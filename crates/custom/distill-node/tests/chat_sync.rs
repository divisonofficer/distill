use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::post;
use distill_node::chat_store::{ChatEvent, Store};
use distill_node::chat_sync::{MAX_SYNC_BODY_BYTES, PeerSync};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tower::ServiceExt as _;

fn private_config(path: &Path, value: &Value) -> PathBuf {
    let config = path.join("peers.json");
    std::fs::write(&config, serde_json::to_vec(value).unwrap()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    config
}

fn add_local_branch(store: &Store, message_id: &str, text: &str) {
    store
        .append(ChatEvent::Create {
            chat_id: "shared".into(),
            title: "Shared".into(),
        })
        .unwrap();
    store
        .append(ChatEvent::Message {
            chat_id: "shared".into(),
            message_id: message_id.into(),
            parent_id: None,
            role: "user".into(),
            text: text.into(),
            model: None,
        })
        .unwrap();
}

fn peer(
    identity: &distill_node::chat_store::Identity,
    incoming: &str,
    outgoing: &str,
    url: String,
) -> Value {
    json!({
        "nodeId": identity.node_id,
        "publicKey": identity.public_key,
        "incomingToken": incoming,
        "outgoingToken": outgoing,
        "outgoingUrl": url,
    })
}

fn request(body: Vec<u8>, token: Option<&str>, origin: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder()
        .method("POST")
        .uri("/api/chat-sync")
        .header("content-type", "application/json");
    if let Some(token) = token {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    if let Some(origin) = origin {
        builder = builder.header("origin", origin);
    }
    builder.body(Body::from(body)).unwrap()
}

async fn status(app: Router, request: Request<Body>) -> StatusCode {
    app.oneshot(request).await.unwrap().status()
}

#[tokio::test]
async fn two_loopback_peer_routers_converge_and_sync_routes_reject_untrusted_input() {
    let left_dir = tempfile::tempdir().unwrap();
    let right_dir = tempfile::tempdir().unwrap();
    let left_store = Store::open(&left_dir.path().join("store")).unwrap();
    let right_store = Store::open(&right_dir.path().join("store")).unwrap();
    add_local_branch(&left_store, "left-root", "left branch");
    add_local_branch(&right_store, "right-root", "right branch");

    let left_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let right_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let left_url = format!(
        "http://{}/api/chat-sync",
        left_listener.local_addr().unwrap()
    );
    let right_url = format!(
        "http://{}/api/chat-sync",
        right_listener.local_addr().unwrap()
    );
    let left_token = "left-incoming-secret-01234567890";
    let right_token = "right-incoming-secret-01234567890";

    let left_config = private_config(
        left_dir.path(),
        &json!({"peers": [peer(&right_store.identity(), left_token, right_token, right_url)]}),
    );
    let right_config = private_config(
        right_dir.path(),
        &json!({"peers": [peer(&left_store.identity(), right_token, left_token, left_url)]}),
    );
    let left_sync = PeerSync::load(&left_config, left_store.clone()).unwrap();
    let right_sync = PeerSync::load(&right_config, right_store.clone()).unwrap();
    let left_app = left_sync.routes();
    let right_app = right_sync.routes();
    let left_server = tokio::spawn(async move {
        let _ = axum::serve(left_listener, left_app).await;
    });
    let right_server = tokio::spawn(async move {
        let _ = axum::serve(right_listener, right_app).await;
    });

    assert_eq!(
        status(
            left_sync.routes(),
            request(br#"{"events":[]}"#.to_vec(), None, None),
        )
        .await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        status(
            left_sync.routes(),
            request(br#"{"events":[]}"#.to_vec(), Some("wrong-token"), None),
        )
        .await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        status(
            left_sync.routes(),
            request(
                br#"{"events":[]}"#.to_vec(),
                Some(left_token),
                Some("https://browser.example")
            ),
        )
        .await,
        StatusCode::FORBIDDEN
    );

    let before = right_store.export().len();
    let mut forged = left_store.export();
    forged[0].signature = "invalid-signature".into();
    assert_eq!(
        status(
            right_sync.routes(),
            request(
                serde_json::to_vec(&json!({"events": forged})).unwrap(),
                Some(right_token),
                None,
            ),
        )
        .await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(right_store.export().len(), before);

    let unknown_author = json!({
        "author": "0".repeat(64),
        "seq": 1,
        "previous_hash": null,
        "event": {"type":"create", "chat_id":"untrusted", "title":"bad"},
        "signature": "AA",
    });
    assert_eq!(
        status(
            right_sync.routes(),
            request(
                serde_json::to_vec(&json!({"events": [unknown_author]})).unwrap(),
                Some(right_token),
                None,
            ),
        )
        .await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(right_store.export().len(), before);

    let duplicate = left_store.export();
    let duplicate_event = duplicate[0].clone();
    assert_eq!(
        status(
            right_sync.routes(),
            request(
                serde_json::to_vec(&json!({"events": [duplicate_event.clone(), duplicate_event]}))
                    .unwrap(),
                Some(right_token),
                None,
            ),
        )
        .await,
        StatusCode::OK
    );
    assert_eq!(right_store.export().len(), before + 1);

    assert_eq!(
        status(
            left_sync.routes(),
            request(vec![b'x'; MAX_SYNC_BODY_BYTES + 1], Some(left_token), None),
        )
        .await,
        StatusCode::PAYLOAD_TOO_LARGE
    );

    let (left_stop, left_shutdown) = tokio::sync::watch::channel(false);
    let (right_stop, right_shutdown) = tokio::sync::watch::channel(false);
    let left_worker = tokio::spawn(async move { left_sync.run(left_shutdown).await });
    let right_worker = tokio::spawn(async move { right_sync.run(right_shutdown).await });
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if left_store.export().len() == 4 && right_store.export().len() == 4 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("peer snapshots converge");
    for store in [&left_store, &right_store] {
        let conversation = store.conversation("shared").unwrap();
        assert!(
            conversation
                .messages
                .iter()
                .any(|message| message.message_id == "left-root")
        );
        assert!(
            conversation
                .messages
                .iter()
                .any(|message| message.message_id == "right-root")
        );
    }
    left_stop.send(true).unwrap();
    right_stop.send(true).unwrap();
    left_worker.await.unwrap();
    right_worker.await.unwrap();
    left_server.abort();
    right_server.abort();
}

#[tokio::test]
async fn peer_client_does_not_follow_redirects() {
    let local_dir = tempfile::tempdir().unwrap();
    let remote_dir = tempfile::tempdir().unwrap();
    let local_store = Store::open(&local_dir.path().join("store")).unwrap();
    let remote_store = Store::open(&remote_dir.path().join("store")).unwrap();
    let redirect_target_hits = Arc::new(AtomicUsize::new(0));
    let target_hits = redirect_target_hits.clone();
    let redirect_app = Router::new()
        .route(
            "/api/chat-sync",
            post(|| async { (StatusCode::TEMPORARY_REDIRECT, [("location", "/target")]) }),
        )
        .route(
            "/target",
            post(move || {
                let hits = target_hits.clone();
                async move {
                    hits.fetch_add(1, Ordering::SeqCst);
                    axum::Json(json!({"events": []}))
                }
            }),
        );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/api/chat-sync", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, redirect_app).await;
    });
    let config = private_config(
        local_dir.path(),
        &json!({"peers": [peer(&remote_store.identity(), "incoming-token-012345678901234567890", "outgoing-token-012345678901234567890", url)]}),
    );
    let sync = PeerSync::load(&config, local_store).unwrap();
    let status = sync.status();
    let (stop, shutdown) = tokio::sync::watch::channel(false);
    let worker = tokio::spawn(async move { sync.run(shutdown).await });
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if status.snapshot().last_attempt_unix_seconds.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("peer exchange attempted");
    assert_eq!(redirect_target_hits.load(Ordering::SeqCst), 0);
    assert_eq!(status.snapshot().last_error, Some("peer_exchange_failed"));
    stop.send(true).unwrap();
    worker.await.unwrap();
    server.abort();
}

#[tokio::test]
async fn peer_bearer_tokens_require_32_ascii_characters() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("store")).unwrap();
    let remote_dir = tempfile::tempdir().unwrap();
    let remote = Store::open(&remote_dir.path().join("store")).unwrap();
    let invalid = private_config(
        dir.path(),
        &json!({"peers": [peer(
            &remote.identity(),
            &"i".repeat(31),
            &"o".repeat(32),
            "https://peer.example/api/chat-sync".into(),
        )]}),
    );
    assert!(PeerSync::load(&invalid, store.clone()).is_err());
    let valid = private_config(
        dir.path(),
        &json!({"peers": [peer(
            &remote.identity(),
            &"i".repeat(32),
            &"o".repeat(32),
            "https://peer.example/api/chat-sync".into(),
        )]}),
    );
    assert!(PeerSync::load(&valid, store.clone()).is_ok());
    let non_ascii = private_config(
        dir.path(),
        &json!({"peers": [peer(
            &remote.identity(),
            &format!("{}é", "i".repeat(31)),
            &"o".repeat(32),
            "https://peer.example/api/chat-sync".into(),
        )]}),
    );
    assert!(PeerSync::load(&non_ascii, store).is_err());
}

#[tokio::test]
async fn peer_config_rejects_non_loopback_http_and_public_permissions() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("store")).unwrap();
    let remote_dir = tempfile::tempdir().unwrap();
    let remote = Store::open(&remote_dir.path().join("store")).unwrap();
    let invalid = private_config(
        dir.path(),
        &json!({"peers": [peer(&remote.identity(), "incoming-token-012345678901234567890", "outgoing-token-012345678901234567890", "http://example.com/api/chat-sync".into())]}),
    );
    assert!(PeerSync::load(&invalid, store.clone()).is_err());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&invalid, std::fs::Permissions::from_mode(0o644)).unwrap();
        let valid_url = json!({"peers": [peer(&remote.identity(), "incoming-token-012345678901234567890", "outgoing-token-012345678901234567890", "https://peer.example/api/chat-sync".into())]});
        std::fs::write(&invalid, serde_json::to_vec(&valid_url).unwrap()).unwrap();
        assert!(PeerSync::load(&invalid, store).is_err());
    }
}
