use std::{path::Path, sync::Arc, time::Duration};

use axum::{Extension, Router, middleware};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use distill_node::{
    chat_store::{ChatEvent, Store},
    chat_sync::PeerSync,
    connections::Connections,
    guard::{self, GuardConfig},
};
use serde_json::{Value, json};
use tokio::net::TcpListener;

fn app(connections: Connections, sync: PeerSync, origin: &str) -> Router {
    let guard = Arc::new(GuardConfig {
        owner: "owner".into(),
        allowed_origins: vec![origin.into()],
    });
    Router::new()
        .merge(
            connections
                .owner_routes()
                .layer(middleware::from_fn_with_state(guard, guard::guard)),
        )
        .merge(connections.public_routes())
        .merge(sync.routes())
        .layer(Extension(connections))
}

fn load(dir: &Path, store: Store, origin: &str) -> (Connections, PeerSync) {
    let sync = PeerSync::empty(store.clone()).unwrap();
    let connections =
        Connections::load_or_create(dir, store, sync.clone(), vec![origin.into()], String::new())
            .unwrap();
    (connections, sync)
}

fn add_branch(store: &Store, id: &str) {
    store
        .append(ChatEvent::Create {
            chat_id: id.into(),
            title: id.into(),
        })
        .unwrap();
    store
        .append(ChatEvent::Message {
            chat_id: id.into(),
            message_id: id.into(),
            parent_id: None,
            role: "user".into(),
            text: id.into(),
            model: None,
            text_files: Vec::new(),
        })
        .unwrap();
}

async fn round(sync: PeerSync) {
    let status = sync.status();
    let (stop, shutdown) = tokio::sync::watch::channel(false);
    let worker = tokio::spawn(async move { sync.run(shutdown).await });
    tokio::time::timeout(Duration::from_secs(5), async {
        while status.snapshot().last_attempt_unix_seconds.is_none() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    stop.send(true).unwrap();
    worker.await.unwrap();
}

#[tokio::test]
async fn pairing_sync_restart_and_revoke() {
    let left_dir = tempfile::tempdir().unwrap();
    let right_dir = tempfile::tempdir().unwrap();
    let left_store = Store::open(&left_dir.path().join("store")).unwrap();
    let right_store = Store::open(&right_dir.path().join("store")).unwrap();
    let left_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let right_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let left_origin = format!("http://{}", left_listener.local_addr().unwrap());
    let right_origin = format!("http://{}", right_listener.local_addr().unwrap());
    let (left, left_sync) = load(left_dir.path(), left_store.clone(), &left_origin);
    let (right, right_sync) = load(right_dir.path(), right_store.clone(), &right_origin);
    let left_server = tokio::spawn(
        axum::serve(
            left_listener,
            app(left.clone(), left_sync.clone(), &left_origin),
        )
        .into_future(),
    );
    let right_server = tokio::spawn(
        axum::serve(
            right_listener,
            app(right.clone(), right_sync.clone(), &right_origin),
        )
        .into_future(),
    );
    let client = reqwest::Client::new();
    let owner_post = |url: String, origin: &str, body: Value| {
        client
            .post(url)
            .header("Tailscale-User-Login", "owner")
            .header("Origin", origin)
            .json(&body)
    };
    let invite_url = format!("{left_origin}/api/connections/invite");
    assert_eq!(
        owner_post(
            invite_url.clone(),
            &left_origin,
            json!({"origin":"https://bad.example"})
        )
        .send()
        .await
        .unwrap()
        .status(),
        403
    );
    let invitation: Value = owner_post(invite_url, &left_origin, json!({"origin":left_origin}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let encoded = invitation["invitation"].as_str().unwrap();
    let decoded: Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(encoded).unwrap()).unwrap();
    assert_eq!(decoded["nodeId"], left_store.identity().node_id);
    assert_eq!(decoded["publicKey"], left_store.identity().public_key);
    assert!(decoded.get("node_id").is_none());
    let join_url = format!("{right_origin}/api/connections/join");
    let mut expired = decoded.clone();
    expired["expiresAt"] = json!(0);
    let encode = |value: &Value| URL_SAFE_NO_PAD.encode(serde_json::to_vec(value).unwrap());
    assert_eq!(
        owner_post(
            join_url.clone(),
            &right_origin,
            json!({"origin":right_origin,"invitation":encode(&expired)})
        )
        .send()
        .await
        .unwrap()
        .status(),
        400
    );
    let mut mismatch = decoded.clone();
    mismatch["publicKey"] = json!(right_store.identity().public_key);
    assert_eq!(
        owner_post(
            join_url.clone(),
            &right_origin,
            json!({"origin":right_origin,"invitation":encode(&mismatch)})
        )
        .send()
        .await
        .unwrap()
        .status(),
        400
    );
    assert_eq!(
        owner_post(
            join_url,
            &right_origin,
            json!({"origin":right_origin,"invitation":encoded})
        )
        .send()
        .await
        .unwrap()
        .status(),
        200
    );
    let accept = json!({"token":decoded["token"],"origin":right_origin,"nodeId":right_store.identity().node_id,"publicKey":right_store.identity().public_key,"incomingToken":"x".repeat(32)});
    assert_eq!(
        client
            .post(format!("{left_origin}/api/chat-pair/accept"))
            .json(&accept)
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    assert!(left.allows_origin(&right_origin));
    assert!(right.allows_origin(&left_origin));
    let config: Value = serde_json::from_slice(
        &std::fs::read(left_dir.path().join("managed-chat-peers.json")).unwrap(),
    )
    .unwrap();
    let token = config["peers"][0]["incomingToken"].as_str().unwrap();
    for (url, remote) in [(&left_origin, &right_origin), (&right_origin, &left_origin)] {
        let endpoint = format!("{url}/api/connections");
        assert_eq!(
            client
                .get(&endpoint)
                .header("Origin", remote)
                .send()
                .await
                .unwrap()
                .status(),
            403
        );
        let response = client
            .get(&endpoint)
            .header("Origin", remote)
            .header("Tailscale-User-Login", "owner")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let body: Value = response.json().await.unwrap();
        assert!(body["identity"]["nodeId"].is_string());
        assert!(body["identity"]["publicKey"].is_string());
        assert!(body["sync"].is_object());
        let text = body.to_string();
        assert!(!text.contains(token));
        assert!(
            !text.contains("incomingToken")
                && !text.contains("outgoingToken")
                && !text.contains("privateKey")
        );
    }
    add_branch(&left_store, "left");
    add_branch(&right_store, "right");
    round(right_sync.clone()).await;
    assert_eq!(left_store.export().len(), 4);
    assert_eq!(right_store.export().len(), 4);
    left_server.abort();
    right_server.abort();
    let _ = left_server.await;
    let _ = right_server.await;
    let (left, left_sync) = load(left_dir.path(), left_store.clone(), &left_origin);
    let (right, right_sync) = load(right_dir.path(), right_store.clone(), &right_origin);
    assert!(left.allows_origin(&right_origin) && right.allows_origin(&left_origin));
    let left_listener = TcpListener::bind(left_origin.trim_start_matches("http://"))
        .await
        .unwrap();
    let right_listener = TcpListener::bind(right_origin.trim_start_matches("http://"))
        .await
        .unwrap();
    let left_server = tokio::spawn(
        axum::serve(left_listener, app(left.clone(), left_sync, &left_origin)).into_future(),
    );
    let right_server = tokio::spawn(
        axum::serve(
            right_listener,
            app(right, right_sync.clone(), &right_origin),
        )
        .into_future(),
    );
    assert_eq!(
        owner_post(
            format!("{left_origin}/api/connections/remove"),
            &left_origin,
            json!({"nodeId":right_store.identity().node_id})
        )
        .send()
        .await
        .unwrap()
        .status(),
        200
    );
    assert!(!left.allows_origin(&right_origin));
    add_branch(&right_store, "after-revoke");
    assert_eq!(
        client
            .post(format!("{left_origin}/api/chat-sync"))
            .bearer_auth(token)
            .json(&json!({"events":right_store.export()}))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    let before = left_store.export().len();
    round(right_sync.clone()).await;
    assert_eq!(
        right_sync.status().snapshot().last_error,
        Some("peer_exchange_failed")
    );
    assert_eq!(left_store.export().len(), before);
    left_server.abort();
    right_server.abort();
}
