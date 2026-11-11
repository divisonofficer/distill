//! Request guards for the trusted listener: Origin allowlist and JSON-only state changes.
//! Public API: plan/research-base/CONTRACT.md §5.

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::{HeaderMap, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::TAILSCALE_USER_LOGIN;

#[derive(Clone)]
pub struct GuardConfig {
    pub owner: String,
    pub allowed_origins: Vec<String>,
}

/// Owner identity, Origin allowlist, WebSocket Origin, and JSON-only writes.
pub async fn guard(State(cfg): State<Arc<GuardConfig>>, req: Request, next: Next) -> Response {
    let headers = req.headers();

    let login = headers
        .get(TAILSCALE_USER_LOGIN)
        .and_then(|v| v.to_str().ok());
    if login != Some(cfg.owner.as_str()) {
        return (StatusCode::FORBIDDEN, "not the node owner").into_response();
    }

    let origin = headers.get(header::ORIGIN);
    let origin_ok = origin
        .and_then(|v| v.to_str().ok())
        .is_some_and(|o| cfg.allowed_origins.iter().any(|a| a == o));
    if (origin.is_some() || is_websocket_upgrade(headers)) && !origin_ok {
        return (StatusCode::FORBIDDEN, "origin not allowed").into_response();
    }

    let is_write = matches!(
        *req.method(),
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    );
    if is_write && !is_json(headers) {
        return (
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "expected application/json",
        )
            .into_response();
    }

    next.run(req).await
}

fn is_websocket_upgrade(headers: &HeaderMap) -> bool {
    headers
        .get_all(header::UPGRADE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .any(|token| token.trim().eq_ignore_ascii_case("websocket"))
}

fn is_json(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .and_then(|ct| ct.split(';').next())
        .is_some_and(|media| media.trim().eq_ignore_ascii_case("application/json"))
}
