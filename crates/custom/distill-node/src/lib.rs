//! Postudio node — per-host gateway (plan/research-base/PLAN.md §4).
//!
//! Trusted listener: web app, session list, board and session WebSockets, Web Push,
//! all behind the owner/Origin guard. Untrusted listener: session file and port proxy.

pub mod accounts;
pub mod agent;
pub mod board;
pub mod guard;
pub mod interaction;
pub mod keeper;
pub mod listen;
pub mod models;
pub mod push;
pub mod reduce;
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
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct NodeInfo {
    pub host: String,
    pub version: &'static str,
}

/// Start the keeper and push sender, then serve both listeners until one fails.
pub async fn run(cfg: NodeConfig) -> anyhow::Result<()> {
    let keeper = Keeper::start(cfg.agent.clone()).await?;
    let push = Arc::new(Push::load_or_create(&cfg.state_dir)?);
    tokio::spawn(notify_needs_input(keeper.clone(), push.clone()));

    let guard = Arc::new(GuardConfig {
        owner: cfg.owner.clone(),
        allowed_origins: cfg.allowed_origins.clone(),
    });
    let trusted = trusted_router(
        keeper.clone(),
        push,
        cfg.agent.clone(),
        guard,
        cfg.web_dir.clone(),
    );
    let resolver: untrusted::CwdResolver = {
        let keeper = keeper.clone();
        Arc::new(move |sid: &str| keeper.cwd_of(sid))
    };
    tokio::try_join!(
        listen::serve(cfg.listen, trusted),
        listen::serve(cfg.untrusted_listen, untrusted::routes(resolver)),
    )?;
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
) -> Router {
    let guarded = Router::new()
        .route("/api/node", get(node_info))
        .merge(accounts::routes(agent.clone()))
        .merge(board::routes(keeper.clone()))
        .merge(models::routes(keeper.clone()))
        .merge(session::routes(agent, keeper))
        .merge(push.routes())
        .layer(middleware::from_fn_with_state(guard, guard::guard));
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .merge(guarded)
        .fallback(move |req: Request| {
            let web_dir = web_dir.clone();
            async move { serve_web(web_dir.as_deref(), req).await }
        })
}

async fn node_info() -> Json<NodeInfo> {
    Json(NodeInfo {
        host: gethostname::gethostname().to_string_lossy().into_owned(),
        version: distill_version::VERSION,
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
