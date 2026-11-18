#![cfg(unix)]

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use distill_node::agent::AgentConfig;
use distill_node::guard::GuardConfig;
use distill_node::keeper::Keeper;
use distill_node::push::Push;
use distill_node::trusted_router;
use futures_util::{SinkExt as _, StreamExt as _};
use serde_json::{Value, json};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;
use tokio_tungstenite::tungstenite::handshake::client::Request as WsRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::{Message, Utf8Bytes};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

const OWNER: &str = "model-test@example.com";
const ORIGIN: &str = "https://model-test.example.com";
const WAIT: Duration = Duration::from_secs(8);
const FAKE: &str = r###"
import json, os, sys

log_path = os.environ["FAKE_LOG"]
cwd = os.environ["FAKE_CWD"]
sessions = [{"sessionId": "s1", "cwd": cwd, "activity": "idle"}]
model_calls = 0

def send(value):
    sys.stdout.write(json.dumps(value) + "\n")
    sys.stdout.flush()

def ok(request_id, result):
    send({"jsonrpc": "2.0", "id": request_id, "result": result})

def fail(request_id, message):
    send({"jsonrpc": "2.0", "id": request_id, "error": {"code": -32602, "message": message}})

for line in sys.stdin:
    request = json.loads(line)
    with open(log_path, "a") as log:
        log.write(json.dumps(request) + "\n")
    if "method" not in request:
        continue
    request_id, method, params = request.get("id"), request["method"], request.get("params", {})
    if method == "initialize":
        ok(request_id, {"authMethods": [{"id": "xai.api_key"}]})
    elif method == "authenticate":
        ok(request_id, {})
    elif method == "_x.ai/sessions/list":
        ok(request_id, {"sessions": sessions})
    elif method == "_x.ai/models/list":
        model_calls += 1
        if model_calls == 1:
            ok(request_id, {"result": {"currentModelId": "catalog-model", "availableModels": [
                {"modelId": "catalog-model", "name": "Catalog Model", "_meta": {"hidden": True}},
                {"modelId": "other-model", "name": "Other Model"}
            ]}})
        else:
            ok(request_id, {"result": None, "error": "catalog unavailable"})
    elif method == "session/new":
        sessions.append({"sessionId": "s2", "cwd": params["cwd"], "activity": "idle"})
        ok(request_id, {"sessionId": "s2"})
    elif method == "session/load":
        ok(request_id, {"models": {"currentModelId": "initial-model", "availableModels": []}})
    elif method == "session/set_model":
        if params["modelId"] == "reject-model":
            fail(request_id, "unknown model")
        else:
            send({"jsonrpc": "2.0", "method": "_x.ai/session_notification", "params": {
                "sessionId": params["sessionId"], "update": {
                    "sessionUpdate": "model_changed", "model_id": "notification-model"
                }
            }})
            ok(request_id, {"_meta": {"canonicalModelId": "canonical-model"}})
    else:
        ok(request_id, {})
"###;

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

struct Fixture {
    base: String,
    addr: SocketAddr,
    log: PathBuf,
    cwd: PathBuf,
    _dir: tempfile::TempDir,
}

async fn start() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("fake_agent.py");
    std::fs::write(&script, FAKE).unwrap();
    let log = dir.path().join("fake.log");
    let cwd = dir.path().join("cwd");
    std::fs::create_dir(&cwd).unwrap();
    let agent = AgentConfig {
        distill_bin: PathBuf::from("python3"),
        args: vec![script.into_os_string()],
        extra_env: vec![
            ("FAKE_LOG".into(), log.clone().into_os_string()),
            ("FAKE_CWD".into(), cwd.clone().into_os_string()),
        ],
    };
    let keeper = Keeper::start(agent.clone()).await.unwrap();
    let state_dir = dir.path().join("state");
    let push = Arc::new(Push::load_or_create(&state_dir).unwrap());
    let chat_store = distill_node::chat_store::Store::open(&state_dir.join("chats")).unwrap();
    let guard = Arc::new(GuardConfig {
        owner: OWNER.to_owned(),
        allowed_origins: vec![ORIGIN.to_owned()],
    });
    let app = trusted_router(keeper, push, agent, guard, None, chat_store);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Fixture {
        base: format!("http://{addr}"),
        addr,
        log,
        cwd,
        _dir: dir,
    }
}

#[allow(
    clippy::disallowed_methods,
    reason = "localhost-only test client against the node bound on 127.0.0.1"
)]
fn client() -> reqwest::Client {
    reqwest::Client::new()
}

fn ws_request(addr: SocketAddr) -> WsRequest {
    let mut request = format!("ws://{addr}/ws/session/s1")
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("Tailscale-User-Login", HeaderValue::from_static(OWNER));
    request
        .headers_mut()
        .insert("Origin", HeaderValue::from_static(ORIGIN));
    request
}

async fn ws(f: &Fixture) -> Ws {
    let (socket, _) = timeout(WAIT, connect_async(ws_request(f.addr)))
        .await
        .unwrap()
        .unwrap();
    socket
}

async fn next_json(socket: &mut Ws) -> Value {
    loop {
        let frame = timeout(WAIT, socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        if let Message::Text(text) = frame {
            return serde_json::from_str(&text).unwrap();
        }
    }
}

async fn until(socket: &mut Ws, event: &str) -> Value {
    loop {
        let value = next_json(socket).await;
        if value["t"] == event {
            return value;
        }
    }
}

async fn send(socket: &mut Ws, value: Value) {
    socket
        .send(Message::Text(Utf8Bytes::from(value.to_string())))
        .await
        .unwrap();
}

fn requests(path: &PathBuf) -> Vec<Value> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[tokio::test]
async fn models_catalog_create_and_session_selection() {
    let f = start().await;
    let http = client();
    let catalog: Value = http
        .get(format!("{}/api/models", f.base))
        .header("Tailscale-User-Login", OWNER)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        catalog,
        json!({
            "currentModelId": "catalog-model",
            "availableModels": [
                {"modelId": "catalog-model", "name": "Catalog Model"},
                {"modelId": "other-model", "name": "Other Model"}
            ]
        })
    );
    let catalog_error = http
        .get(format!("{}/api/models", f.base))
        .header("Tailscale-User-Login", OWNER)
        .send()
        .await
        .unwrap();
    assert_eq!(catalog_error.status(), reqwest::StatusCode::BAD_GATEWAY);
    assert_eq!(
        catalog_error.json::<Value>().await.unwrap()["error"],
        "catalog unavailable"
    );

    let invalid = http
        .post(format!("{}/api/sessions", f.base))
        .header("Tailscale-User-Login", OWNER)
        .json(&json!({"cwd": f.cwd, "modelId": "  "}))
        .send()
        .await
        .unwrap();
    assert_eq!(invalid.status(), reqwest::StatusCode::BAD_REQUEST);

    let created = http
        .post(format!("{}/api/sessions", f.base))
        .header("Tailscale-User-Login", OWNER)
        .json(&json!({"cwd": f.cwd, "modelId": "requested-model"}))
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), reqwest::StatusCode::OK);
    let new_request = requests(&f.log)
        .into_iter()
        .find(|request| request["method"] == "session/new")
        .unwrap();
    assert_eq!(new_request["params"]["_meta"]["modelId"], "requested-model");

    let mut socket = ws(&f).await;
    let initial = until(&mut socket, "session.model").await;
    assert_eq!(initial["model_id"], "initial-model");

    send(&mut socket, json!({"t": "model", "model_id": ""})).await;
    assert!(
        until(&mut socket, "error").await["message"]
            .as_str()
            .unwrap()
            .contains("must not be empty")
    );

    send(
        &mut socket,
        json!({"t": "model", "model_id": "requested-model"}),
    )
    .await;
    assert_eq!(
        until(&mut socket, "session.model").await["model_id"],
        "notification-model"
    );
    assert_eq!(
        until(&mut socket, "session.model").await["model_id"],
        "canonical-model"
    );

    send(
        &mut socket,
        json!({"t": "model", "model_id": "reject-model"}),
    )
    .await;
    assert!(
        until(&mut socket, "error").await["message"]
            .as_str()
            .unwrap()
            .contains("unknown model")
    );

    let sent_models: Vec<Value> = requests(&f.log)
        .into_iter()
        .filter(|request| request["method"] == "session/set_model")
        .collect();
    assert_eq!(sent_models.len(), 2);
    assert_eq!(sent_models[0]["params"]["sessionId"], "s1");
    assert_eq!(sent_models[0]["params"]["modelId"], "requested-model");
    assert_eq!(sent_models[1]["params"]["modelId"], "reject-model");
}
