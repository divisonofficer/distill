//! Postudio node — per-host gateway (plan/research-base/PLAN.md §4).
//!
//! WP1 skeleton: health, host info, and the owner check that every `/api` route
//! sits behind. `tailscale serve` terminates HTTPS in front of this server and
//! adds `Tailscale-User-Login`; the node itself only binds loopback.
//! The ACP multiplexer (WP2) and the web app (WP3) build on this router.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;

/// Header `tailscale serve` sets to the requesting user's login for tailnet traffic.
pub const TAILSCALE_USER_LOGIN: &str = "Tailscale-User-Login";

#[derive(Debug, Clone)]
pub struct NodeConfig {
    /// Tailscale login allowed to use `/api`. `None` disables the check (local development only).
    pub owner_login: Option<String>,
    /// Local Distill agent (`distill agent serve`) WebSocket URL.
    pub agent_url: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct NodeInfo {
    pub hostname: String,
    pub os: &'static str,
    pub arch: &'static str,
    pub version: &'static str,
    pub agent_url: String,
}

impl NodeInfo {
    fn current(config: &NodeConfig) -> Self {
        Self {
            hostname: gethostname::gethostname().to_string_lossy().into_owned(),
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
            version: distill_version::VERSION,
            agent_url: config.agent_url.clone(),
        }
    }
}

pub fn router(config: NodeConfig) -> Router {
    let config = Arc::new(config);
    let api = Router::new()
        .route("/api/node", get(node_info))
        .route_layer(middleware::from_fn_with_state(
            config.clone(),
            require_owner,
        ));
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .merge(api)
        .with_state(config)
}

async fn node_info(State(config): State<Arc<NodeConfig>>) -> Json<NodeInfo> {
    Json(NodeInfo::current(&config))
}

/// Reject `/api` requests whose Tailscale identity is not the configured owner.
/// A missing header means the request did not come through `tailscale serve`.
async fn require_owner(
    State(config): State<Arc<NodeConfig>>,
    request: Request,
    next: Next,
) -> Response {
    let Some(owner) = config.owner_login.as_deref() else {
        return next.run(request).await;
    };
    let login = request
        .headers()
        .get(TAILSCALE_USER_LOGIN)
        .and_then(|v| v.to_str().ok());
    if login == Some(owner) {
        next.run(request).await
    } else {
        tracing::warn!(?login, "rejected /api request: not the node owner");
        (StatusCode::FORBIDDEN, "not the node owner").into_response()
    }
}

/// Serve until the listener fails or `shutdown` resolves.
pub async fn serve(
    listener: tokio::net::TcpListener,
    config: NodeConfig,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> anyhow::Result<()> {
    let addr: SocketAddr = listener.local_addr()?;
    tracing::info!(%addr, "postudio node listening");
    axum::serve(listener, router(config))
        .with_graceful_shutdown(shutdown)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn spawn(config: NodeConfig) -> (String, tokio::sync::oneshot::Sender<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        tokio::spawn(serve(listener, config, async {
            let _ = rx.await;
        }));
        (base, tx)
    }

    #[allow(
        clippy::disallowed_methods,
        reason = "localhost-only test client against the node bound on 127.0.0.1"
    )]
    fn local_client() -> reqwest::Client {
        reqwest::Client::new()
    }

    fn config(owner: Option<&str>) -> NodeConfig {
        NodeConfig {
            owner_login: owner.map(str::to_owned),
            agent_url: "ws://127.0.0.1:2419/ws".to_owned(),
        }
    }

    #[tokio::test]
    async fn healthz_needs_no_identity() {
        let (base, _stop) = spawn(config(Some("me@example.com"))).await;
        let res = local_client()
            .get(format!("{base}/healthz"))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), 200);
        assert_eq!(res.text().await.unwrap(), "ok");
    }

    #[tokio::test]
    async fn api_rejects_missing_or_foreign_identity() {
        let (base, _stop) = spawn(config(Some("me@example.com"))).await;
        let client = local_client();
        let missing = client.get(format!("{base}/api/node")).send().await.unwrap();
        assert_eq!(missing.status(), 403);
        let foreign = client
            .get(format!("{base}/api/node"))
            .header(TAILSCALE_USER_LOGIN, "someone@else.com")
            .send()
            .await
            .unwrap();
        assert_eq!(foreign.status(), 403);
    }

    #[tokio::test]
    async fn api_node_reports_host_for_owner() {
        let (base, _stop) = spawn(config(Some("me@example.com"))).await;
        let res = local_client()
            .get(format!("{base}/api/node"))
            .header(TAILSCALE_USER_LOGIN, "me@example.com")
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), 200);
        let body: serde_json::Value = res.json().await.unwrap();
        assert_eq!(body["os"], std::env::consts::OS);
        assert_eq!(body["agent_url"], "ws://127.0.0.1:2419/ws");
        assert!(!body["hostname"].as_str().unwrap_or_default().is_empty());
    }
}
