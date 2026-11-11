//! Postudio node against the real distill leader and the mock model.
//! Needs GROK_BINARY: `cargo test -p postudio-probe --test node -- --ignored --nocapture --test-threads=1`.

use std::net::TcpListener;
use std::time::Duration;

use futures_util::{SinkExt as _, StreamExt as _};
use postudio_probe::stack::{self, Stack};
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio::time::{Instant, timeout};
use tokio_tungstenite::tungstenite::Error as WsError;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::handshake::client::Request;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

const OWNER: &str = "me@example.com";
const ORIGIN: &str = "http://127.0.0.1:5173";
const WAIT: Duration = Duration::from_secs(30);

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// A free port; the node also needs port + 1.
#[allow(
    clippy::disallowed_methods,
    reason = "localhost-only test client against the node on 127.0.0.1"
)]
fn http() -> reqwest::Client {
    reqwest::Client::new()
}

fn get(stack: &Stack, path: &str) -> reqwest::RequestBuilder {
    http()
        .get(format!("{}{path}", stack.url))
        .header("Tailscale-User-Login", OWNER)
        .header("Origin", ORIGIN)
}

fn post_json(stack: &Stack, path: &str, body: Value) -> reqwest::RequestBuilder {
    http()
        .post(format!("{}{path}", stack.url))
        .header("Tailscale-User-Login", OWNER)
        .header("Origin", ORIGIN)
        .json(&body)
}

fn ws_request(stack: &Stack, path: &str, origin: &'static str) -> Request {
    let ws_url = stack.url.replacen("http", "ws", 1);
    let mut req = format!("{ws_url}{path}")
        .into_client_request()
        .expect("ws request");
    req.headers_mut()
        .insert("Tailscale-User-Login", HeaderValue::from_static(OWNER));
    req.headers_mut()
        .insert("Origin", HeaderValue::from_static(origin));
    req
}

async fn create_session(stack: &Stack) -> String {
    let resp = post_json(
        stack,
        "/api/sessions",
        json!({"cwd": stack.cwd.display().to_string(), "mode": "ask"}),
    )
    .send()
    .await
    .expect("POST /api/sessions");
    let status = resp.status();
    let body: Value = resp.json().await.expect("POST /api/sessions json");
    assert!(status.is_success(), "POST /api/sessions {status}: {body}");
    body["sessionId"]
        .as_str()
        .unwrap_or_else(|| panic!("no sessionId in {body}"))
        .to_owned()
}

/// One WebSocket plus every frame read from it.
struct Feed {
    ws: Socket,
    log: Vec<Value>,
}

impl Feed {
    async fn open(stack: &Stack, path: &str) -> Feed {
        let (ws, _) = timeout(WAIT, connect_async(ws_request(stack, path, ORIGIN)))
            .await
            .expect("ws connect timed out")
            .unwrap_or_else(|e| panic!("ws {path}: {e}"));
        Feed {
            ws,
            log: Vec::new(),
        }
    }

    async fn send(&mut self, value: Value) {
        self.ws
            .send(Message::Text(value.to_string().into()))
            .await
            .unwrap_or_else(|e| panic!("send {value}: {e}"));
    }

    /// Reads frames until one satisfies `pred`. Every frame read is kept in `log`.
    async fn until(&mut self, what: &str, pred: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + WAIT;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match timeout(remaining, self.ws.next()).await {
                Ok(Some(Ok(Message::Text(text)))) => {
                    let frame: Value = serde_json::from_str(text.as_str())
                        .unwrap_or_else(|e| panic!("{what}: bad json frame {text}: {e}"));
                    self.log.push(frame);
                    let last = self.log.last().expect("just pushed");
                    if pred(last) {
                        return last.clone();
                    }
                }
                Ok(Some(Ok(_))) => {}
                Ok(Some(Err(e))) => {
                    panic!("{what}: socket error {e}; last frames: {}", tail(&self.log))
                }
                Ok(None) => panic!("{what}: socket closed; last frames: {}", tail(&self.log)),
                Err(_) => panic!(
                    "{what}: timed out after {WAIT:?}; last frames: {}",
                    tail(&self.log)
                ),
            }
        }
    }

    async fn close(mut self) {
        let _ = self.ws.close(None).await;
    }
}

fn tail(log: &[Value]) -> String {
    let start = log.len().saturating_sub(5);
    format!("{:?}", &log[start..])
}

fn is_type(frame: &Value, t: &str) -> bool {
    frame["t"] == t
}

fn needs_input_for(frame: &Value, sid: &str) -> bool {
    is_type(frame, "board.upsert")
        && frame["sessions"].as_array().is_some_and(|rows| {
            rows.iter()
                .any(|r| r["sessionId"] == sid && r["activity"] == "needs_input")
        })
}

fn deltas(log: &[Value]) -> String {
    log.iter()
        .filter(|f| is_type(f, "message.delta"))
        .filter_map(|f| f["text"].as_str())
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the distill binary; set GROK_BINARY"]
async fn end_to_end_permission_flow() {
    let stack = stack::start(0, None).await.expect("start stack");
    let sid = create_session(&stack).await;
    let mut board = Feed::open(&stack, "/ws/board").await;
    let mut session = Feed::open(&stack, &format!("/ws/session/{sid}")).await;
    session
        .until("replay.end", |f| is_type(f, "replay.end"))
        .await;

    session.send(json!({"t": "prompt", "text": "go"})).await;
    let perm = session
        .until("permission.request", |f| is_type(f, "permission.request"))
        .await;
    board
        .until("board row needs_input", |f| needs_input_for(f, &sid))
        .await;

    session
        .send(json!({"t": "permission", "id": perm["id"], "option_id": "allow-once"}))
        .await;
    let end = session.until("turn.end", |f| is_type(f, "turn.end")).await;
    assert_eq!(end["stop_reason"], "end_turn", "turn.end: {end}");
    let text = deltas(&session.log);
    assert!(
        text.contains("README"),
        "no message.delta with README; deltas {text:?}; last frames {}",
        tail(&session.log)
    );
    let readme = std::fs::read_to_string(stack.cwd.join("README.md")).expect("read README.md");
    assert_eq!(readme, "hello from postudio");
    board.close().await;
    session.close().await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the distill binary; set GROK_BINARY"]
async fn second_client_sees_the_same_turn() {
    let stack = stack::start(0, None).await.expect("start stack");
    let sid = create_session(&stack).await;
    let mut a = Feed::open(&stack, &format!("/ws/session/{sid}")).await;
    a.until("replay.end (A)", |f| is_type(f, "replay.end"))
        .await;
    let mut b = Feed::open(&stack, &format!("/ws/session/{sid}")).await;
    b.until("replay.end (B)", |f| is_type(f, "replay.end"))
        .await;

    a.send(json!({"t": "prompt", "text": "go"})).await;
    a.until("permission.request (A)", |f| {
        is_type(f, "permission.request")
    })
    .await;
    let perm = b
        .until("permission.request (B)", |f| {
            is_type(f, "permission.request")
        })
        .await;
    b.send(json!({"t": "permission", "id": perm["id"], "option_id": "allow-once"}))
        .await;

    let end_a = a.until("turn.end (A)", |f| is_type(f, "turn.end")).await;
    let end_b = b.until("turn.end (B)", |f| is_type(f, "turn.end")).await;
    assert_eq!(end_a["stop_reason"], "end_turn", "A: {end_a}");
    assert_eq!(end_b["stop_reason"], "end_turn", "B: {end_b}");
    a.close().await;
    b.close().await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the distill binary; set GROK_BINARY"]
async fn keeper_keeps_session_resident() {
    let stack = stack::start(0, None).await.expect("start stack");
    let sid = create_session(&stack).await;
    let board = Feed::open(&stack, "/ws/board").await;
    let mut session = Feed::open(&stack, &format!("/ws/session/{sid}")).await;
    session
        .until("replay.end", |f| is_type(f, "replay.end"))
        .await;
    board.close().await;
    session.close().await;

    tokio::time::sleep(Duration::from_secs(3)).await;
    let resp = get(&stack, "/api/sessions")
        .send()
        .await
        .expect("GET /api/sessions");
    let status = resp.status();
    let body: Value = resp.json().await.expect("GET /api/sessions json");
    assert!(status.is_success(), "GET /api/sessions {status}: {body}");
    let row = body["sessions"]
        .as_array()
        .and_then(|rows| rows.iter().find(|r| r["sessionId"] == sid.as_str()))
        .unwrap_or_else(|| panic!("session {sid} not listed: {body}"));
    assert_eq!(row["resident"], true, "row: {row}");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the distill binary; set GROK_BINARY"]
async fn rejects_without_login_and_bad_origin() {
    let stack = stack::start(0, None).await.expect("start stack");

    let no_login = http()
        .get(format!("{}/api/sessions", stack.url))
        .header("Origin", ORIGIN)
        .send()
        .await
        .expect("GET without login");
    assert_eq!(no_login.status(), 403, "GET without login");

    match connect_async(ws_request(&stack, "/ws/board", "http://evil.example")).await {
        Err(WsError::Http(resp)) => {
            assert_eq!(resp.status(), 403, "ws with bad origin")
        }
        Err(other) => panic!("ws with bad origin: expected HTTP 403, got {other}"),
        Ok(_) => panic!("ws with bad origin: upgrade succeeded"),
    }
}
