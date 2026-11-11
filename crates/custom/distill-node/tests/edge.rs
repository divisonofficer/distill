//! Edge security: guard, listener setup, and the untrusted router.

use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use axum::middleware;
use axum::routing::get;
use distill_node::guard::{GuardConfig, guard};
use distill_node::listen::{ListenAddr, serve};
use distill_node::untrusted::{CwdResolver, routes};
use tower::ServiceExt;

const OWNER: &str = "me@example.com";
const ORIGIN: &str = "https://node.example.com";

fn guarded() -> Router {
    let cfg = Arc::new(GuardConfig {
        owner: OWNER.to_owned(),
        allowed_origins: vec![ORIGIN.to_owned()],
    });
    Router::new()
        .route("/api/node", get(|| async { "ok" }).post(|| async { "ok" }))
        .route("/ws/session", get(|| async { "ws" }))
        .layer(middleware::from_fn_with_state(cfg, guard))
}

async fn send(app: Router, req: Request<Body>) -> StatusCode {
    app.oneshot(req).await.unwrap().status()
}

fn get_req(uri: &str) -> axum::http::request::Builder {
    Request::builder()
        .method(Method::GET)
        .uri(uri)
        .header("Tailscale-User-Login", OWNER)
}

#[tokio::test]
async fn owner_ok_and_missing_or_foreign_login_forbidden() {
    let req = get_req("/api/node").body(Body::empty()).unwrap();
    assert_eq!(send(guarded(), req).await, StatusCode::OK);

    let req = Request::get("/api/node").body(Body::empty()).unwrap();
    assert_eq!(send(guarded(), req).await, StatusCode::FORBIDDEN);

    let req = Request::get("/api/node")
        .header("Tailscale-User-Login", "other@example.com")
        .body(Body::empty())
        .unwrap();
    assert_eq!(send(guarded(), req).await, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn origin_must_be_allowed_when_present() {
    let forged = get_req("/api/node")
        .header(header::ORIGIN, "https://evil.example.net")
        .body(Body::empty())
        .unwrap();
    assert_eq!(send(guarded(), forged).await, StatusCode::FORBIDDEN);

    let allowed = get_req("/api/node")
        .header(header::ORIGIN, ORIGIN)
        .body(Body::empty())
        .unwrap();
    assert_eq!(send(guarded(), allowed).await, StatusCode::OK);
}

#[tokio::test]
async fn websocket_upgrade_requires_allowed_origin() {
    let no_origin = get_req("/ws/session")
        .header(header::CONNECTION, "Upgrade")
        .header(header::UPGRADE, "websocket")
        .body(Body::empty())
        .unwrap();
    assert_eq!(send(guarded(), no_origin).await, StatusCode::FORBIDDEN);

    let with_origin = get_req("/ws/session")
        .header(header::CONNECTION, "Upgrade")
        .header(header::UPGRADE, "websocket")
        .header(header::ORIGIN, ORIGIN)
        .body(Body::empty())
        .unwrap();
    assert_eq!(send(guarded(), with_origin).await, StatusCode::OK);
}

#[tokio::test]
async fn writes_require_json_content_type() {
    let text = Request::post("/api/node")
        .header("Tailscale-User-Login", OWNER)
        .header(header::CONTENT_TYPE, "text/plain")
        .body(Body::from("x"))
        .unwrap();
    assert_eq!(
        send(guarded(), text).await,
        StatusCode::UNSUPPORTED_MEDIA_TYPE
    );

    let json = Request::post("/api/node")
        .header("Tailscale-User-Login", OWNER)
        .header(header::CONTENT_TYPE, "application/json; charset=utf-8")
        .body(Body::from("{}"))
        .unwrap();
    assert_eq!(send(guarded(), json).await, StatusCode::OK);
}

fn resolver(cwd: PathBuf) -> CwdResolver {
    Arc::new(move |id: &str| (id == "s1").then(|| cwd.clone()))
}

fn untrusted(cwd: PathBuf) -> Router {
    routes(resolver(cwd))
}

#[tokio::test]
async fn untrusted_router_has_no_api_or_ws() {
    let dir = tempfile::tempdir().unwrap();
    let req = Request::get("/api/node").body(Body::empty()).unwrap();
    assert_eq!(
        send(untrusted(dir.path().to_path_buf()), req).await,
        StatusCode::NOT_FOUND
    );
    let req = Request::get("/ws/session").body(Body::empty()).unwrap();
    assert_eq!(
        send(untrusted(dir.path().to_path_buf()), req).await,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn file_serving_is_confined_to_session_cwd() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("cwd");
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::write(root.join("sub/index.html"), "<p>hi</p>").unwrap();
    std::fs::write(dir.path().join("secret.txt"), "nope").unwrap();

    let file = Request::get("/f/s1/sub/index.html")
        .body(Body::empty())
        .unwrap();
    let res = untrusted(root.clone()).oneshot(file).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        res.headers()[header::CONTENT_TYPE],
        "text/html; charset=utf-8"
    );
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(&body[..], b"<p>hi</p>");

    for uri in [
        "/f/s1/../secret.txt",
        "/f/s1/sub/../../secret.txt",
        "/f/s2/sub/index.html",
        "/f/s1/sub",
    ] {
        let req = Request::get(uri).body(Body::empty()).unwrap();
        assert_eq!(
            send(untrusted(root.clone()), req).await,
            StatusCode::NOT_FOUND,
            "{uri}"
        );
    }
}

#[cfg(unix)]
#[tokio::test]
async fn file_symlink_escape_is_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("cwd");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(dir.path().join("secret.txt"), "nope").unwrap();
    std::os::unix::fs::symlink(dir.path().join("secret.txt"), root.join("link.txt")).unwrap();

    let req = Request::get("/f/s1/link.txt").body(Body::empty()).unwrap();
    assert_eq!(send(untrusted(root), req).await, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn port_proxy_forwards_body_and_drops_tailscale_headers() {
    let upstream = Router::new().route(
        "/hello",
        get(|headers: axum::http::HeaderMap| async move {
            let seen = headers.contains_key("tailscale-user-login");
            format!("upstream:{seen}")
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let _ = axum::serve(listener, upstream).await;
    });

    let dir = tempfile::tempdir().unwrap();
    let req = get_req(&format!("/p/{port}/hello?x=1"))
        .body(Body::empty())
        .unwrap();
    let res = untrusted(dir.path().to_path_buf())
        .oneshot(req)
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(&body[..], b"upstream:false");
}

#[tokio::test]
async fn port_proxy_rejects_bad_port_and_maps_connect_failure_to_502() {
    let dir = tempfile::tempdir().unwrap();
    let req = Request::get("/p/0/").body(Body::empty()).unwrap();
    assert_eq!(
        send(untrusted(dir.path().to_path_buf()), req).await,
        StatusCode::BAD_REQUEST
    );

    // Bind then drop to get a port with no listener.
    let closed = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let req = Request::get(format!("/p/{closed}/"))
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        send(untrusted(dir.path().to_path_buf()), req).await,
        StatusCode::BAD_GATEWAY
    );
}

#[tokio::test]
async fn port_proxy_websocket_upgrade_is_not_implemented() {
    let dir = tempfile::tempdir().unwrap();
    let req = Request::get("/p/8080/ws")
        .header(header::CONNECTION, "Upgrade")
        .header(header::UPGRADE, "websocket")
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        send(untrusted(dir.path().to_path_buf()), req).await,
        StatusCode::NOT_IMPLEMENTED
    );
}

#[test]
fn listen_addr_parses_tcp_and_unix() {
    assert!(matches!(
        "tcp:127.0.0.1:7410".parse::<ListenAddr>(),
        Ok(ListenAddr::Tcp(_))
    ));
    #[cfg(unix)]
    assert!(matches!(
        "unix:/tmp/x.sock".parse::<ListenAddr>(),
        Ok(ListenAddr::Unix(_))
    ));
    assert!("http:127.0.0.1:1".parse::<ListenAddr>().is_err());
}

#[tokio::test]
async fn tcp_serve_refuses_non_loopback() {
    let addr: ListenAddr = "tcp:0.0.0.0:0".parse().unwrap();
    assert!(serve(addr, Router::new()).await.is_err());
}

#[cfg(unix)]
#[tokio::test]
async fn unix_serve_creates_socket_in_0700_dir() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let parent = dir.path().join("postudio");
    std::fs::create_dir_all(&parent).unwrap();
    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o755)).unwrap();
    let sock = parent.join("node.sock");
    std::fs::write(&sock, "stale").unwrap();

    let task = tokio::spawn(serve(ListenAddr::Unix(sock.clone()), Router::new()));
    for _ in 0..100 {
        if tokio::net::UnixStream::connect(&sock).await.is_ok() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let connected = tokio::net::UnixStream::connect(&sock).await.is_ok();
    task.abort();

    assert!(connected, "socket not accepting connections");
    let mode = std::fs::metadata(&parent).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o700);
}
