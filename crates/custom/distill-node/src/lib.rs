//! Postudio node — per-host gateway (plan/research-base/PLAN.md §4).
//!
//! Trusted listener: web app, session list, board and session WebSockets, Web Push,
//! all behind the owner/Origin guard. Untrusted listener: session file and port proxy.

pub mod account_login;
pub mod accounts;
pub mod agent;
pub mod artifacts;
pub mod attachments;
pub mod board;
pub mod chat_store;
pub mod chat_sync;
pub mod chats;
pub mod configuration;
pub mod connections;
pub mod executions;
pub mod fleet;
pub mod guard;
pub mod interaction;
pub mod keeper;
pub mod listen;
pub mod models;
pub mod ports;
pub mod push;
pub mod reduce;
pub mod research;
pub(crate) mod research_chat;
pub(crate) mod research_context;
pub(crate) mod research_gateway;
pub(crate) mod research_library;
pub(crate) mod research_literature;
pub(crate) mod research_mcp;
pub(crate) mod research_project;
pub(crate) mod research_refs;
pub(crate) mod research_sessions;
pub(crate) mod research_targets;
pub mod sep;
pub mod session;
pub mod untrusted;

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use axum::Router;
use axum::extract::Request;
use axum::http::{Method, StatusCode, header};
use axum::middleware;
use axum::response::{IntoResponse, Json, Response};
use axum::routing::get;
use serde::Serialize;
use serde_json::json;

use agent::AgentConfig;
use configuration::{listener_ports, validate_node_config};
use guard::GuardConfig;
use keeper::Keeper;
use listen::ListenAddr;
use push::Push;

/// Header `tailscale serve` sets to the requesting user's login for tailnet traffic.
pub const TAILSCALE_USER_LOGIN: &str = "Tailscale-User-Login";

#[derive(Debug, Clone)]
pub struct NodeConfig {
    pub owner: String,
    pub listen: ListenAddr,
    pub untrusted_listen: ListenAddr,
    pub allowed_origins: Vec<String>,
    pub agent: AgentConfig,
    pub state_dir: PathBuf,
    pub web_dir: Option<PathBuf>,
    pub chat_peer_config: Option<PathBuf>,
    pub untrusted_origin: Option<String>,
    pub openresearch_bin: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct NodeInfo {
    pub host: String,
    pub version: &'static str,
    #[serde(rename = "untrustedOrigin", skip_serializing_if = "Option::is_none")]
    pub untrusted_origin: Option<String>,
}

/// Start the keeper and push sender, then serve both listeners until one fails.
pub async fn run(cfg: NodeConfig) -> anyhow::Result<()> {
    validate_node_config(&cfg)?;
    let bindings = if cfg.openresearch_bin.is_some() {
        let endpoint = match &cfg.listen {
            ListenAddr::Tcp(addr) => {
                research_sessions::NodeEndpoint::Http(format!("http://{addr}"))
            }
            #[cfg(unix)]
            ListenAddr::Unix(path) => research_sessions::NodeEndpoint::Unix(path.clone()),
        };
        Some(research_sessions::ResearchBindings::open(
            &cfg.state_dir,
            endpoint,
        )?)
    } else {
        None
    };
    let keeper =
        Keeper::start_with_mcp(cfg.agent.clone(), bindings.as_ref().map(|b| b.resolver())).await?;
    let push = Arc::new(Push::load_or_create(&cfg.state_dir)?);
    let chat_store = chat_store::Store::open(&cfg.state_dir.join("chats"))?;
    let sync = match cfg.chat_peer_config.as_deref() {
        Some(path) => chat_sync::PeerSync::load(path, chat_store.clone())?,
        None => chat_sync::PeerSync::empty(chat_store.clone())?,
    };
    let connections = connections::Connections::load_or_create(
        &cfg.state_dir,
        chat_store.clone(),
        sync.clone(),
        cfg.allowed_origins.clone(),
        cfg.untrusted_origin.clone().unwrap_or_default(),
    )?;
    let peer_sync = Some(sync);
    let completion: Arc<dyn chats::ChatCompleter> =
        Arc::new(chats::AcpChatCompleter::new(keeper.clone()));
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let mut background = tokio::task::JoinSet::new();
    if let Some(sync) = peer_sync.clone() {
        background.spawn(async move { sync.run(shutdown_rx).await });
    }
    background.spawn(notify_needs_input(keeper.clone(), push.clone()));

    let guard = Arc::new(GuardConfig {
        owner: cfg.owner.clone(),
        allowed_origins: cfg.allowed_origins.clone(),
    });
    let blocked_ports = ports::port_policy(listener_ports(&cfg));
    for port in research::existing_runtime_ports(&cfg.state_dir)
        .await
        .map_err(anyhow::Error::msg)?
    {
        blocked_ports
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(port);
    }
    let research = research::ResearchService::new(
        cfg.openresearch_bin.clone(),
        cfg.state_dir.clone(),
        blocked_ports.clone(),
    );
    let gateway = match std::env::var("POSTUDIO_RESEARCH_DASHBOARD_ORIGIN") {
        Ok(origin) if bindings.is_some() => {
            let origin = url::Url::parse(&origin)?;
            let dashboard_host = origin
                .host_str()
                .ok_or_else(|| anyhow::anyhow!("Dashboard hostname required"))?;
            anyhow::ensure!(
                !cfg.allowed_origins
                    .iter()
                    .chain(cfg.untrusted_origin.iter())
                    .filter_map(|value| url::Url::parse(value).ok())
                    .any(|value| value.host_str() == Some(dashboard_host)),
                "Dashboard must use a different hostname from trusted and untrusted applications; cookies have no port scope"
            );
            anyhow::ensure!(
                origin.scheme() == "https",
                "Dashboard requires a dedicated HTTPS origin"
            );
            let address: ListenAddr = std::env::var("POSTUDIO_RESEARCH_DASHBOARD_LISTEN")
                .map_err(|_| anyhow::anyhow!("Dashboard listener required"))?
                .parse()?;
            if let ListenAddr::Tcp(addr) = &address {
                anyhow::ensure!(addr.port() != 0, "Choose a fixed private dashboard port");
                blocked_ports
                    .write()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(addr.port());
            }
            let gw = research_gateway::DashboardGateway::new(origin, research.clone())?;
            let app = gw.router();
            background.spawn(async move {
                if let Err(error) = listen::serve(address, app).await {
                    tracing::error!(%error,"research dashboard stopped");
                }
            });
            Some(gw)
        }
        _ => None,
    };
    let trusted = trusted_services_router(
        keeper.clone(),
        push,
        cfg.agent.clone(),
        guard,
        cfg.web_dir.clone(),
        chat_store,
        (Some(completion), peer_sync.clone()),
        WebConfig {
            untrusted_origin: cfg.untrusted_origin.clone(),
            blocked_ports: blocked_ports.clone(),
            research: Some(research),
            bindings,
            gateway,
            connections: Some(connections),
        },
    );
    let resolver: untrusted::CwdResolver = {
        let keeper = keeper.clone();
        Arc::new(move |sid: &str| keeper.cwd_of(sid))
    };
    let result = tokio::try_join!(
        listen::serve(cfg.listen, trusted),
        listen::serve(
            cfg.untrusted_listen,
            untrusted::routes_with_policy(resolver, blocked_ports)
        ),
    );
    let _ = shutdown_tx.send(true);
    result?;
    Ok(())
}

async fn notify_needs_input(keeper: Keeper, push: Arc<Push>) {
    let mut needs = keeper.needs_input();
    loop {
        let event = match needs.recv().await {
            Ok(event) => event,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
        };
        let row = keeper
            .rows()
            .into_iter()
            .find(|r| r.get("sessionId").and_then(|v| v.as_str()) == Some(&event.session_id));
        let title = row
            .as_ref()
            .and_then(|r| r.get("title").and_then(|v| v.as_str()))
            .filter(|t| !t.is_empty())
            .map(str::to_owned)
            .or_else(|| {
                keeper
                    .cwd_of(&event.session_id)?
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
            })
            .unwrap_or_else(|| event.session_id.clone());
        let payload = json!({
            "title": format!("{title} 승인 대기"),
            "body": event.kind,
            "url": format!("/s/{}", event.session_id),
            "tag": event.session_id,
        });
        push.notify_all(&payload).await;
    }
}

/// Trusted listener. `/healthz` and the static web app are unguarded; `/api` and `/ws` are guarded.
pub fn trusted_router(
    keeper: Keeper,
    push: Arc<Push>,
    agent: AgentConfig,
    guard: Arc<GuardConfig>,
    web_dir: Option<PathBuf>,
    chat_store: chat_store::Store,
) -> Router {
    trusted_router_with_chat_services(keeper, push, agent, guard, web_dir, chat_store, None, None)
}

pub fn trusted_router_with_chat_services(
    keeper: Keeper,
    push: Arc<Push>,
    agent: AgentConfig,
    guard: Arc<GuardConfig>,
    web_dir: Option<PathBuf>,
    chat_store: chat_store::Store,
    completion: Option<Arc<dyn chats::ChatCompleter>>,
    peer_sync: Option<chat_sync::PeerSync>,
) -> Router {
    trusted_services_router(
        keeper,
        push,
        agent,
        guard,
        web_dir,
        chat_store,
        (completion, peer_sync),
        WebConfig::default(),
    )
}

#[derive(Default)]
struct WebConfig {
    connections: Option<connections::Connections>,
    untrusted_origin: Option<String>,
    blocked_ports: ports::PortPolicy,
    research: Option<research::ResearchService>,
    bindings: Option<research_sessions::ResearchBindings>,
    gateway: Option<research_gateway::DashboardGateway>,
}

fn trusted_services_router(
    keeper: Keeper,
    push: Arc<Push>,
    agent: AgentConfig,
    guard: Arc<GuardConfig>,
    web_dir: Option<PathBuf>,
    chat_store: chat_store::Store,
    services: (
        Option<Arc<dyn chats::ChatCompleter>>,
        Option<chat_sync::PeerSync>,
    ),
    web: WebConfig,
) -> Router {
    let (completion, peer_sync) = services;
    let sync_status = peer_sync
        .as_ref()
        .map(chat_sync::PeerSync::status)
        .unwrap_or_else(chat_sync::SyncStatus::disabled);
    let connections = web.connections;
    let connection_routes = connections
        .as_ref()
        .map(|c| c.owner_routes())
        .unwrap_or_default();
    let research = web.research.unwrap_or_else(|| {
        research::ResearchService::new(None, PathBuf::new(), web.blocked_ports.clone())
    });
    let prompt_context = web
        .bindings
        .as_ref()
        .map(|bindings| research_context::ResearchContext {
            service: research.clone(),
            bindings: bindings.clone(),
        });
    let private = web
        .bindings
        .as_ref()
        .map(|b| research_mcp::private_routes(research.clone(), b.clone()))
        .unwrap_or_default();
    let chat = web
        .bindings
        .map(|b| research_chat::routes(research.clone(), b, keeper.clone()))
        .unwrap_or_default();
    let dashboard = Router::new().route(
        "/api/research/projects/{id}/dashboard",
        axum::routing::post(
            move |axum::extract::Path(id): axum::extract::Path<String>,
                  Json(body): Json<serde_json::Value>| async move {
                let _ = body;
                let Some(gateway) = web.gateway else {
                    return (
                        StatusCode::SERVICE_UNAVAILABLE,
                        "Configure a dedicated HTTPS dashboard origin and private listener",
                    )
                        .into_response();
                };
                match gateway.ticket(&id).await {
                    Ok(url) => Json(json!({"url":url})).into_response(),
                    Err(response) => response,
                }
            },
        ),
    );
    let guarded = Router::new()
        .merge(chat)
        .merge(dashboard)
        .merge(research_project::routes(research.clone()))
        .merge(research_targets::routes(research.clone()))
        .merge(research_literature::routes(research.clone()))
        .merge(connection_routes)
        .route(
            "/api/node",
            get(move || node_info(web.untrusted_origin.clone())),
        )
        .merge(fleet::routes())
        .merge(artifacts::routes(keeper.clone()))
        .merge(ports::routes_with_policy(web.blocked_ports.clone()))
        .merge(chats::routes_with_peers(
            chat_store,
            completion.clone(),
            sync_status,
            peer_sync.clone(),
        ))
        .merge(accounts::routes(agent.clone()))
        .merge(account_login::routes(agent.clone()))
        .merge(board::routes(keeper.clone()))
        .merge(models::routes(keeper.clone()))
        .merge(executions::routes(keeper.clone()))
        .merge(research::routes_for_service(research))
        .merge(match prompt_context {
            Some(context) => session::routes_with_context(agent, keeper, context),
            None => session::routes(agent, keeper),
        })
        .merge(push.routes())
        .layer(middleware::from_fn_with_state(guard, guard::guard));
    let mut app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .merge(guarded)
        .merge(private);
    if let Some(peer_sync) = peer_sync {
        app = app.merge(peer_sync.routes());
        if let Some(completion) = completion {
            app = app.merge(peer_sync.completion_routes(completion));
        }
    }
    if let Some(connections) = connections {
        app = app
            .merge(connections.public_routes())
            .layer(axum::Extension(connections));
    }
    app.fallback(move |req: Request| {
        let web_dir = web_dir.clone();
        async move { serve_web(web_dir.as_deref(), req).await }
    })
}

async fn node_info(untrusted_origin: Option<String>) -> Json<NodeInfo> {
    Json(NodeInfo {
        host: gethostname::gethostname().to_string_lossy().into_owned(),
        version: distill_version::VERSION,
        untrusted_origin,
    })
}

/// GET only. Existing files under `root` are served; any other path gets `index.html` (SPA).
async fn serve_web(root: Option<&Path>, req: Request) -> Response {
    let not_found = || StatusCode::NOT_FOUND.into_response();
    let Some(root) = root else {
        return not_found();
    };
    if req.method() != Method::GET {
        return not_found();
    }
    let path = req.uri().path();
    if ["/api", "/ws"]
        .iter()
        .any(|p| path == *p || path.starts_with(&format!("{p}/")))
    {
        return not_found();
    }
    let Ok(decoded) = urlencoding::decode(path) else {
        return not_found();
    };
    let rel = Path::new(decoded.trim_start_matches('/'));
    if rel.components().any(|c| !matches!(c, Component::Normal(_))) {
        return not_found();
    }
    match static_file(root, rel).await {
        Some(res) => res,
        None => static_file(root, Path::new("index.html"))
            .await
            .unwrap_or_else(not_found),
    }
}

/// `rel` under `root` as a regular file; symlinks that leave `root` are refused.
async fn static_file(root: &Path, rel: &Path) -> Option<Response> {
    let root = dunce::canonicalize(root).ok()?;
    let file = dunce::canonicalize(root.join(rel)).ok()?;
    if !file.starts_with(&root) || !tokio::fs::metadata(&file).await.ok()?.is_file() {
        return None;
    }
    let bytes = tokio::fs::read(&file).await.ok()?;
    Some(([(header::CONTENT_TYPE, content_type(&file))], bytes).into_response())
}

fn content_type(path: &Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    match ext.as_deref() {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("json") => "application/json",
        Some("webmanifest") => "application/manifest+json",
        Some("ico") => "image/x-icon",
        Some("txt") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// Run the stdio transport for a single research conversation.
pub async fn research_stdio(
    url: Option<String>,
    socket: Option<PathBuf>,
    grant: String,
) -> anyhow::Result<()> {
    let endpoint = match (url, socket) {
        (Some(url), None) => {
            let u = url::Url::parse(&url)?;
            anyhow::ensure!(
                u.scheme() == "http"
                    && u.host_str()
                        .is_some_and(|host| matches!(host, "127.0.0.1" | "[::1]"))
                    && u.path() == "/"
                    && u.username().is_empty()
                    && u.password().is_none()
                    && u.query().is_none()
                    && u.fragment().is_none(),
                "Expected loopback endpoint"
            );
            research_sessions::NodeEndpoint::Http(url)
        }
        (None, Some(path)) if path.is_absolute() => research_sessions::NodeEndpoint::Unix(path),
        _ => anyhow::bail!("Choose one local endpoint"),
    };
    research_mcp::run_stdio(endpoint, grant).await
}
