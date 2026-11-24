#![cfg(unix)]

use axum::{
    Json, Router,
    extract::{Path, Query},
    routing::{get, post},
};
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use ring::{digest, hmac};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tempfile::TempDir;
use tokio::net::TcpListener;
use tower::ServiceExt;

#[derive(Clone)]
struct Fake {
    seen: Arc<Mutex<Vec<(String, Value)>>>,
}

async fn hello(Query(q): Query<HashMap<String, String>>) -> Json<Value> {
    let nonce = q.get("nonce").unwrap();
    let mut message = b"inst\0".to_vec();
    message.extend_from_slice(nonce.as_bytes());
    let key = hmac::Key::new(hmac::HMAC_SHA256, b"secret");
    let proof = STANDARD.encode(hmac::sign(&key, &message).as_ref());
    Json(json!({"instance":"inst","proto":2,"postudioContract":1,"proof":proof}))
}
async fn projects() -> Json<Value> {
    Json(json!({"projects":[{"id":"p"}]}))
}
async fn project() -> Json<Value> {
    Json(json!({"id":"p"}))
}
async fn experiments() -> Json<Value> {
    Json(json!({"experiments":[{"id":"e"}]}))
}
async fn create_run(
    axum::extract::State(s): axum::extract::State<Fake>,
    Json(v): Json<Value>,
) -> Json<Value> {
    s.seen.lock().unwrap().push(("create".into(), v));
    Json(json!({"id":"r","projectId":"p"}))
}
async fn run(
    Path(id): Path<String>,
    axum::extract::State(s): axum::extract::State<Fake>,
) -> Json<Value> {
    s.seen
        .lock()
        .unwrap()
        .push((format!("run:{id}"), Value::Null));
    Json(json!({"id":id,"projectId":"p"}))
}
async fn cancel(
    Path(id): Path<String>,
    axum::extract::State(s): axum::extract::State<Fake>,
    Json(v): Json<Value>,
) -> Json<Value> {
    s.seen.lock().unwrap().push((format!("cancel:{id}"), v));
    Json(json!({"id":id,"projectId":"p"}))
}
async fn log(
    Path(id): Path<String>,
    Query(q): Query<HashMap<String, String>>,
    axum::extract::State(s): axum::extract::State<Fake>,
) -> Json<Value> {
    s.seen
        .lock()
        .unwrap()
        .push((format!("log:{id}:{}:{}", q["start"], q["end"]), Value::Null));
    Json(json!({"kind":"data","start":1,"ident":"x","data":"/w==","len":3}))
}
async fn files(axum::extract::State(s): axum::extract::State<Fake>) -> Json<Value> {
    s.seen.lock().unwrap().push(("files".into(), Value::Null));
    Json(
        json!({"entries":[{"name":"dir","path":"dir","isDir":true,"children":[{"name":"deep.txt","path":"dir/deep.txt","size":2,"isDir":false}]}]}),
    )
}

async fn setup() -> (TempDir, axum::Router, Arc<Mutex<Vec<(String, Value)>>>) {
    let tmp = tempfile::tempdir().unwrap();
    let state = tmp.path().join("state");
    let root = state.join("research/openresearch");
    let data = root.join("data");
    std::fs::create_dir_all(&data).unwrap();
    let canonical = dunce::canonicalize(&data).unwrap();
    let hash = digest::digest(&digest::SHA256, canonical.to_string_lossy().as_bytes());
    let name: String = hash.as_ref()[..16]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let runtime = PathBuf::from(format!("/tmp/orx-{}", unsafe { libc::geteuid() }));
    std::fs::create_dir_all(&runtime).unwrap();
    std::fs::set_permissions(&runtime, std::fs::Permissions::from_mode(0o700)).unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let fake = Fake { seen: seen.clone() };
    let app = Router::new()
        .route("/api/v2/hello", get(hello))
        .route("/api/projects", get(projects))
        .route("/api/projects/{id}", get(project))
        .route("/api/projects/{id}/experiments", get(experiments))
        .route("/api/v2/runs", post(create_run))
        .route("/api/v2/runs/{id}", get(run))
        .route("/api/v2/runs/{id}/cancel", post(cancel))
        .route("/api/v2/runs/{id}/log", get(log))
        .route("/api/projects/{id}/files", get(files))
        .with_state(fake);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let descriptor =
        json!({"instance":"inst","port":port,"version":"0.2.27","proto":2,"token":"secret"});
    std::fs::write(
        runtime.join(format!("{name}.server.json")),
        descriptor.to_string(),
    )
    .unwrap();
    std::fs::set_permissions(
        runtime.join(format!("{name}.server.json")),
        std::fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    (tmp, distill_node::research::routes(None, state), seen)
}
async fn request(app: &Router, method: &str, uri: &str, body: Value) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 2_000_000)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn v2_payload_envelopes_logs_and_recursive_files() {
    let (_tmp, app, seen) = setup().await;
    let (status, result) = request(
        &app,
        "POST",
        "/api/research/projects/p/runs",
        json!({"experimentId":"e","approved":true,"expectedCommit":"a".repeat(40)}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "run response: {result}");
    assert_eq!(result, json!({"run":{"id":"r","projectId":"p"}}));
    let (status, result) = request(
        &app,
        "POST",
        "/api/research/projects/p/runs/r/cancel",
        json!({"approved":true}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "cancel response: {result}");
    assert_eq!(result, json!({"run":{"id":"r","projectId":"p"}}));
    let (status, result) = request(
        &app,
        "GET",
        "/api/research/projects/p/runs/r/log?start=1&end=4",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(result["end"], 2);
    assert_eq!(result["text"], "�");
    let (status, result) = request(&app, "GET", "/api/research/projects/p/files", json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        result["files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["path"] == "dir/deep.txt")
    );
    let got = seen.lock().unwrap();
    assert_eq!(
        got.iter().find(|x| x.0 == "create").unwrap().1,
        json!({"exp_id":"e","expected_commit":"a".repeat(40),"backend":"local","no_container":false,"force":false,"forwarded":true,"telemetry_suppressed":true})
    );
    assert_eq!(got.iter().find(|x| x.0 == "cancel:r").unwrap().1, json!({}));
}

#[tokio::test]
async fn run_rejects_unapproved_and_overflow_log_range() {
    let (_tmp, app, seen) = setup().await;
    let (status, _) = request(
        &app,
        "POST",
        "/api/research/projects/p/runs",
        json!({"experimentId":"e","approved":false}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = request(
        &app,
        "GET",
        "/api/research/projects/p/runs/r/log?start=18446744073709551615",
        json!({}),
    )
    .await;
    assert_ne!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(seen.lock().unwrap().iter().all(|x| x.0 != "create"));
}
