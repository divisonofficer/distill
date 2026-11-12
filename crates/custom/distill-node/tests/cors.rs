use std::sync::Arc;

use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode, header},
    middleware,
    routing::{get, post},
};
use distill_node::{
    TAILSCALE_USER_LOGIN,
    guard::{self, GuardConfig},
};
use tower::ServiceExt;

const ORIGIN: &str = "https://allowed.example";

fn router() -> Router {
    let config = Arc::new(GuardConfig {
        owner: "owner@example.com".into(),
        allowed_origins: vec![ORIGIN.into()],
    });
    Router::new()
        .route("/get", get(|| async { "ok" }))
        .route("/post", post(|| async { "created" }))
        .layer(middleware::from_fn_with_state(config, guard::guard))
}

async fn send(
    method: Method,
    path: &str,
    origin: Option<&str>,
    content_type: Option<&str>,
    owner: Option<&str>,
) -> axum::response::Response {
    let mut request = Request::builder().method(method).uri(path);
    if let Some(origin) = origin {
        request = request.header(header::ORIGIN, origin);
    }
    if let Some(content_type) = content_type {
        request = request.header(header::CONTENT_TYPE, content_type);
    }
    if let Some(owner) = owner {
        request = request.header(TAILSCALE_USER_LOGIN, owner);
    }
    router()
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

fn assert_cors(response: &axum::response::Response, existing_vary: Option<&str>) {
    let headers = response.headers();
    assert_eq!(headers[header::ACCESS_CONTROL_ALLOW_ORIGIN], ORIGIN);
    assert_eq!(headers[header::ACCESS_CONTROL_ALLOW_CREDENTIALS], "true");
    assert_eq!(
        headers[header::VARY],
        existing_vary.map_or("Origin".to_owned(), |v| format!("{v}, Origin"))
    );
}

#[tokio::test]
async fn allowed_get_has_cors_headers_and_preserves_vary() {
    let response = send(
        Method::GET,
        "/get",
        Some(ORIGIN),
        None,
        Some("owner@example.com"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_cors(&response, None);
}

#[tokio::test]
async fn allowed_json_post_has_cors_headers() {
    let response = send(
        Method::POST,
        "/post",
        Some(ORIGIN),
        Some("application/json"),
        Some("owner@example.com"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_cors(&response, None);
}

#[tokio::test]
async fn allowed_origin_error_has_cors_headers() {
    let response = send(
        Method::POST,
        "/post",
        Some(ORIGIN),
        Some("text/plain"),
        Some("owner@example.com"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    assert_cors(&response, None);
}

#[tokio::test]
async fn preflight_has_narrow_allowlist() {
    let response = send(
        Method::OPTIONS,
        "/post",
        Some(ORIGIN),
        None,
        Some("owner@example.com"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_cors(&response, None);
    assert_eq!(
        response.headers()[header::ACCESS_CONTROL_ALLOW_METHODS],
        "GET, POST"
    );
    assert_eq!(
        response.headers()[header::ACCESS_CONTROL_ALLOW_HEADERS],
        "Content-Type"
    );
}

#[tokio::test]
async fn malicious_origin_and_missing_owner_rejected() {
    let malicious = send(
        Method::GET,
        "/get",
        Some("https://evil.example"),
        None,
        Some("owner@example.com"),
    )
    .await;
    assert_eq!(malicious.status(), StatusCode::FORBIDDEN);
    assert!(
        malicious
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .is_none()
    );
    let missing_owner = send(Method::GET, "/get", Some(ORIGIN), None, None).await;
    assert_eq!(missing_owner.status(), StatusCode::FORBIDDEN);
    assert!(
        missing_owner
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .is_none()
    );
}
