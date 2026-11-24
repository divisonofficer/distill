use std::{path::PathBuf, sync::Arc};

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use distill_node::{
    ports::port_policy,
    untrusted::{CwdResolver, routes_with_policy},
};
use tower::ServiceExt;

fn router(policy: distill_node::ports::PortPolicy) -> axum::Router {
    let resolve_cwd: CwdResolver = Arc::new(|_| Some(PathBuf::from("/tmp")));
    routes_with_policy(resolve_cwd, policy)
}

async fn proxy_get(app: axum::Router, port: u16) -> StatusCode {
    app.oneshot(
        Request::get(format!("/p/{port}/"))
            .body(Body::empty())
            .unwrap(),
    )
    .await
    .unwrap()
    .status()
}

#[tokio::test]
async fn shared_policy_blocks_http_and_websocket_without_affecting_other_service() {
    let service = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let service_port = service.local_addr().unwrap().port();
    let service_app =
        axum::Router::new().route("/", axum::routing::get(|| async { "localservice" }));
    tokio::spawn(async move {
        let _ = axum::serve(service, service_app).await;
    });
    let second = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let second_port = second.local_addr().unwrap().port();
    let second_app = axum::Router::new().route("/", axum::routing::get(|| async { "secondapp" }));
    tokio::spawn(async move {
        let _ = axum::serve(second, second_app).await;
    });

    let policy = port_policy([]);
    let app = router(policy.clone());
    assert_eq!(proxy_get(app.clone(), service_port).await, StatusCode::OK);
    assert_eq!(proxy_get(app.clone(), second_port).await, StatusCode::OK);

    policy.write().unwrap().insert(service_port);
    assert_eq!(
        proxy_get(app.clone(), service_port).await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(proxy_get(app.clone(), second_port).await, StatusCode::OK);

    let websocket = Request::get(format!("/p/{service_port}/ws"))
        .header("connection", "Upgrade")
        .header("upgrade", "websocket")
        .header("sec-websocket-version", "13")
        .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        app.oneshot(websocket).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
}
