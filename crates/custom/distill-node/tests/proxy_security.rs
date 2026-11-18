use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use distill_node::untrusted::{CwdResolver, routes_with_prohibited_ports};
use futures_util::{SinkExt, StreamExt};
use tower::ServiceExt;

fn router() -> axum::Router {
    let resolve_cwd: CwdResolver = Arc::new(|_| Some(PathBuf::from("/tmp")));
    routes_with_prohibited_ports(resolve_cwd, [7410, 7411, 8123])
}

#[tokio::test]
async fn prohibited_ports_rejected_for_every_proxy_route_and_method() {
    for path in ["/p/7410", "/p/7411/", "/p/8123/probe"] {
        for method in [
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::DELETE,
            Method::OPTIONS,
        ] {
            let req = Request::builder()
                .method(method)
                .uri(path)
                .body(Body::empty())
                .unwrap();
            let response = router().oneshot(req).await.unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN, "{path}");
        }
    }
}

#[tokio::test]
async fn proxy_headers_cannot_bypass_prohibited_port_check() {
    let req = Request::get("/p/7410/admin")
        .header("x-forwarded-port", "6006")
        .header("host", "127.0.0.1:6006")
        .header("connection", "Upgrade")
        .header("upgrade", "websocket")
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        router().oneshot(req).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn permitted_tensorboard_port_still_proxies() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let upstream = axum::Router::new().route("/", axum::routing::get(|| async { "tensorboard" }));
    tokio::spawn(async move {
        let _ = axum::serve(listener, upstream).await;
    });
    let req = Request::get(format!("/p/{port}/"))
        .body(Body::empty())
        .unwrap();
    let response = router().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        &axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap()[..],
        b"tensorboard"
    );
}

#[tokio::test]
async fn connection_nominated_headers_removed_in_both_directions() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let upstream = axum::Router::new().route(
        "/",
        axum::routing::get(|request: axum::extract::Request| async move {
            assert!(request.headers().get("x-request-private").is_none());
            let mut response = axum::http::Response::new(Body::from("ok"));
            response
                .headers_mut()
                .append("connection", "X-Response-Private".parse().unwrap());
            response
                .headers_mut()
                .append("x-response-private", "secret".parse().unwrap());
            response
                .headers_mut()
                .append("x-response-visible", "yes".parse().unwrap());
            response
        }),
    );
    tokio::spawn(async move {
        let _ = axum::serve(listener, upstream).await;
    });
    let req = Request::get(format!("/p/{port}/"))
        .header("connection", "keep-alive, X-Request-Private")
        .header("x-request-private", "secret")
        .body(Body::empty())
        .unwrap();
    let response = router().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(response.headers().get("x-response-private").is_none());
    assert_eq!(response.headers().get("x-response-visible").unwrap(), "yes");
}

#[tokio::test]
async fn websocket_proxy_preserves_required_subprotocol_and_relays_text_binary() {
    let upstream_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream_port = upstream_listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (stream, _) = upstream_listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_hdr_async(
            stream,
            |request: &axum::http::Request<()>, mut response: axum::http::Response<()>| {
                assert_eq!(
                    request.headers().get("sec-websocket-protocol").unwrap(),
                    "graphql-transport-ws"
                );
                response.headers_mut().insert(
                    "sec-websocket-protocol",
                    "graphql-transport-ws".parse().unwrap(),
                );
                Ok(response)
            },
        )
        .await
        .unwrap();
        for _ in 0..2 {
            let msg = socket.next().await.unwrap().unwrap();
            socket.send(msg).await.unwrap();
        }
        let _ = socket.close(None).await;
    });
    let proxy_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_addr = proxy_listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(proxy_listener, router()).await;
    });
    let mut request =
        tokio_tungstenite::tungstenite::client::IntoClientRequest::into_client_request(format!(
            "ws://{proxy_addr}/p/{upstream_port}/echo?token=abc"
        ))
        .unwrap();
    request.headers_mut().insert(
        "sec-websocket-protocol",
        "graphql-transport-ws".parse().unwrap(),
    );
    let (mut client, response) = tokio_tungstenite::connect_async(request).await.unwrap();
    assert_eq!(
        response.headers().get("sec-websocket-protocol").unwrap(),
        "graphql-transport-ws"
    );
    assert!(response.headers().get("x-private").is_none());
    client
        .send(tokio_tungstenite::tungstenite::Message::Text("ping".into()))
        .await
        .unwrap();
    assert_eq!(
        client.next().await.unwrap().unwrap().into_text().unwrap(),
        "ping"
    );
    client
        .send(tokio_tungstenite::tungstenite::Message::Binary(
            vec![1, 2, 3].into(),
        ))
        .await
        .unwrap();
    assert_eq!(
        client.next().await.unwrap().unwrap().into_data(),
        vec![1, 2, 3]
    );
}
