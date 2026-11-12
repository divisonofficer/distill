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

    let origin = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let origin_ok = origin
        .as_deref()
        .is_some_and(|origin| cfg.allowed_origins.iter().any(|allowed| allowed == origin));
    if (origin.is_some() || is_websocket_upgrade(headers)) && !origin_ok {
        return (StatusCode::FORBIDDEN, "origin not allowed").into_response();
    }

    if req.method() == Method::OPTIONS {
        let mut response = StatusCode::NO_CONTENT.into_response();
        add_cors_headers(&mut response, origin.as_deref());
        if origin.is_some() {
            let headers = response.headers_mut();
            headers.insert(
                header::ACCESS_CONTROL_ALLOW_METHODS,
                "GET, POST".parse().unwrap(),
            );
            headers.insert(
                header::ACCESS_CONTROL_ALLOW_HEADERS,
                "Content-Type".parse().unwrap(),
            );
        }
        return response;
    }

    let is_write = matches!(
        *req.method(),
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    );
    let json_body = is_json(headers);
    let response = if is_write && !json_body {
        (
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "expected application/json",
        )
            .into_response()
    } else {
        next.run(req).await
    };
    let mut response = response;
    add_cors_headers(&mut response, origin.as_deref());
    response
}

fn add_cors_headers(response: &mut Response, origin: Option<&str>) {
    let Some(origin) = origin else {
        return;
    };
    let headers = response.headers_mut();
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        origin.parse().expect("validated origin"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_CREDENTIALS,
        "true".parse().unwrap(),
    );
    let mut vary_values = headers
        .get_all(header::VARY)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(',').map(str::trim))
        .filter(|value| !value.eq_ignore_ascii_case("origin"))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    vary_values.push("Origin".to_owned());
    headers.insert(header::VARY, vary_values.join(", ").parse().unwrap());
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
