//! Postudio node against the real distill leader and the mock model.
//! Needs GROK_BINARY: `cargo test -p postudio-probe --test node -- --ignored --nocapture --test-threads=1`.

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

/// A prompt given at creation runs on the keeper; the permission request must wait for a
/// client that attaches later, and replay plus live must not duplicate the reply.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the distill binary; set GROK_BINARY"]
async fn prompt_at_creation_waits_for_late_client() {
    let stack = stack::start(0, None).await.expect("start stack");
    let resp = post_json(
        &stack,
        "/api/sessions",
        json!({"cwd": stack.cwd.display().to_string(), "mode": "ask", "prompt": "go"}),
    )
    .send()
    .await
    .expect("POST /api/sessions");
    let body: Value = resp.json().await.expect("json");
    let sid = body["sessionId"].as_str().expect("sessionId").to_owned();
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    let readme = std::fs::read_to_string(stack.cwd.join("README.md")).expect("read README.md");
    assert_eq!(readme, "hello", "edit ran before anyone approved");
    let mut session = Feed::open(&stack, &format!("/ws/session/{sid}")).await;
    let perm = session
        .until("permission.request", |f| is_type(f, "permission.request"))
        .await;
    session
        .send(json!({"t": "permission", "id": perm["id"], "option_id": "allow-once"}))
        .await;
    session.until("turn.end", |f| is_type(f, "turn.end")).await;
    let text = deltas(&session.log);
    assert_eq!(
        text.matches("README 수정 완료.").count(),
        1,
        "deltas {text:?}"
    );
    session.close().await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the distill binary; set GROK_BINARY"]
async fn permission_mode_survives_browser_reconnect() {
    let stack = stack::start(0, None).await.expect("stack");
    let sid = create_session(&stack).await;
    let mut session = Feed::open(&stack, &format!("/ws/session/{sid}")).await;
    session
        .until("replay.end", |f| is_type(f, "replay.end"))
        .await;
    session.send(json!({"t":"mode", "mode":"auto"})).await;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let rows: Value = get(&stack, "/api/sessions")
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if rows["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["sessionId"] == sid && r["autoMode"] == true && r["yolo"] == false)
        {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "auto mode missing: {rows}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    session.close().await;
    let mut reopened = Feed::open(&stack, &format!("/ws/session/{sid}")).await;
    reopened
        .until("replay.end", |f| is_type(f, "replay.end"))
        .await;
    let rows: Value = get(&stack, "/api/sessions")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        rows["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["sessionId"] == sid && r["autoMode"] == true)
    );
    reopened.close().await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the distill binary; set GROK_BINARY"]
async fn creates_real_leader_worktree_session() {
    use std::process::Command;

    let stack = stack::start(0, None).await.expect("start stack");
    let run_git = |args: &[&str]| {
        let output = Command::new("git")
            .arg("-C")
            .arg(&stack.cwd)
            .args(args)
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    run_git(&["init"]);
    run_git(&[
        "-c",
        "user.name=Probe Test",
        "-c",
        "user.email=probe@example.test",
        "add",
        ".",
    ]);
    run_git(&[
        "-c",
        "user.name=Probe Test",
        "-c",
        "user.email=probe@example.test",
        "commit",
        "-m",
        "test fixture",
    ]);

    let marker = stack.cwd.join("worktree-marker.txt");
    std::fs::write(&marker, "copied from source\n").expect("write marker");
    run_git(&["add", "worktree-marker.txt"]);
    run_git(&[
        "-c",
        "user.name=Probe Test",
        "-c",
        "user.email=probe@example.test",
        "commit",
        "-m",
        "add marker",
    ]);

    let source = std::fs::canonicalize(&stack.cwd).expect("canonicalize source cwd");
    let resp = post_json(
        &stack,
        "/api/sessions",
        json!({"cwd": stack.cwd.display().to_string(), "worktree": {"name": "probe-worktree"}}),
    )
    .send()
    .await
    .expect("POST /api/sessions worktree");
    let status = resp.status();
    let body: Value = resp.json().await.expect("worktree response json");
    assert!(
        status.is_success(),
        "POST /api/sessions worktree {status}: {body}"
    );
    let sid = body["sessionId"]
        .as_str()
        .unwrap_or_else(|| panic!("no sessionId in {body}"));

    let resp = get(&stack, "/api/sessions")
        .send()
        .await
        .expect("GET /api/sessions");
    let status = resp.status();
    let listing: Value = resp.json().await.expect("GET /api/sessions json");
    assert!(status.is_success(), "GET /api/sessions {status}: {listing}");
    let row = listing["sessions"]
        .as_array()
        .and_then(|rows| rows.iter().find(|row| row["sessionId"] == sid))
        .unwrap_or_else(|| panic!("session {sid} missing from listing: {listing}"));
    let cwd = row["cwd"]
        .as_str()
        .unwrap_or_else(|| panic!("session has no cwd: {row}"));
    assert_ne!(
        cwd,
        source.display().to_string(),
        "session cwd remained source: {row}"
    );
    let target = std::path::Path::new(cwd);
    assert!(target.is_dir(), "worktree directory missing: {target:?}");
    assert_eq!(
        std::fs::read_to_string(target.join("worktree-marker.txt")).expect("read copied marker"),
        "copied from source\n"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the distill binary; set GROK_BINARY"]
async fn node_restart_reconnects_to_persistent_leader_and_history() {
    let mut stack = stack::start(0, None).await.expect("start stack");
    let sid = create_session(&stack).await;
    let mut session = Feed::open(&stack, &format!("/ws/session/{sid}")).await;
    session
        .until("replay.end", |f| is_type(f, "replay.end"))
        .await;

    session.send(json!({"t": "prompt", "text": "go"})).await;
    let permission = session
        .until("permission.request", |f| is_type(f, "permission.request"))
        .await;
    session
        .send(json!({
            "t": "permission",
            "id": permission["id"],
            "option_id": "allow-once"
        }))
        .await;
    session.until("turn.end", |f| is_type(f, "turn.end")).await;
    session.close().await;

    stack.restart_node().await.expect("restart node");
    let listing: Value = get(&stack, "/api/sessions")
        .send()
        .await
        .expect("GET /api/sessions after node restart")
        .json()
        .await
        .expect("session listing json");
    let row = listing["sessions"]
        .as_array()
        .and_then(|rows| rows.iter().find(|row| row["sessionId"] == sid))
        .unwrap_or_else(|| panic!("session {sid} missing after restart: {listing}"));
    assert_eq!(row["sessionId"], sid, "saved session missing: {row}");

    let mut reopened = Feed::open(&stack, &format!("/ws/session/{sid}")).await;
    reopened
        .until("replay.end after restart", |f| is_type(f, "replay.end"))
        .await;
    let page = reopened
        .log
        .iter()
        .find(|frame| frame["t"] == "history.page")
        .unwrap_or_else(|| panic!("no replay history after restart: {}", tail(&reopened.log)));
    assert!(
        page["items"].as_array().is_some_and(|items| items
            .iter()
            .any(|item| item["t"] == "user.message" && item["text"] == "go")),
        "prompt history missing after restart: {page}"
    );
    let listing: Value = get(&stack, "/api/sessions").send().await.unwrap().json().await.unwrap();
    assert!(listing["sessions"].as_array().unwrap().iter().any(|row| row["sessionId"] == sid && row["resident"] == true));
    reopened.close().await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the distill binary; set GROK_BINARY"]
async fn real_subagent_parent_metadata_reaches_node_roster() {
    let stack = stack::start(0, None).await.expect("start stack");
    stack.set_conversations([
        distill_test_support::Conversation::nth(1)
            .calls([distill_test_support::MockToolCall::new(
                distill_test_support::Tool::Task,
                json!({
                    "description": "persist hierarchy probe",
                    "prompt": "Return the exact text CHILD_PROBE_DONE.",
                    "subagent_type": "general-purpose",
                    "run_in_background": false
                }),
            )])
            .reply("Parent received child result."),
        distill_test_support::Conversation::for_system_prompt(
            "Complete the assigned task directly.",
        )
        .reply("CHILD_PROBE_DONE"),
    ]);

    let sid = create_session(&stack).await;
    let mut session = Feed::open(&stack, &format!("/ws/session/{sid}")).await;
    session
        .until("replay.end", |f| is_type(f, "replay.end"))
        .await;
    session
        .send(json!({"t": "prompt", "text": "Delegate the probe."}))
        .await;
    let permission = session
        .until("permission.request", |f| is_type(f, "permission.request"))
        .await;
    session
        .send(json!({
            "t": "permission",
            "id": permission["id"],
            "option_id": "allow-once"
        }))
        .await;
    let end = session.until("turn.end", |f| is_type(f, "turn.end")).await;
    assert_eq!(end["stop_reason"], "end_turn", "turn.end: {end}");
    assert!(
        stack.script_violations().is_empty(),
        "mock script violation: {:?}; frames: {}",
        stack.script_violations(),
        tail(&session.log)
    );
    let conversations = stack.conversations();
    let parent = conversations
        .iter()
        .find(|conversation| conversation.number() == 1)
        .unwrap_or_else(|| panic!("parent model conversation missing: {conversations:?}"));
    assert!(
        parent.saw_tool_call(distill_test_support::Tool::Task),
        "parent did not make real Task tool call: {parent:?}"
    );
    assert!(
        stack
            .conversation_for_system_prompt("Complete the assigned task directly.")
            .is_some(),
        "mock model never opened child conversation; conversations: {conversations:?}; frames: {}",
        tail(&session.log)
    );
    let sessions_dir = stack.distill_home().join("sessions");
    let mut summaries = Vec::new();
    for cwd_entry in std::fs::read_dir(&sessions_dir).expect("read persisted sessions") {
        let cwd_path = cwd_entry.expect("session cwd entry").path();
        if !cwd_path.is_dir() {
            continue;
        }
        for session_entry in std::fs::read_dir(cwd_path).expect("read cwd sessions") {
            let session_path = session_entry.expect("session entry").path();
            if !session_path.is_dir() {
                continue;
            }
            if let Ok(bytes) = std::fs::read(session_path.join("summary.json"))
                && let Ok(summary) = serde_json::from_slice::<Value>(&bytes)
            {
                summaries.push(summary);
            }
        }
    }
    let persisted_child = summaries
        .iter()
        .find(|summary| {
            summary["info"]["id"] != sid
                && summary["session_kind"]
                    .as_str()
                    .is_some_and(|kind| kind.starts_with("subagent"))
        })
        .unwrap_or_else(|| panic!("actual child summary missing: {summaries:?}"));
    let child_id = persisted_child["info"]["id"]
        .as_str()
        .expect("persisted child session id");
    let listing: Value = get(&stack, "/api/sessions")
        .send()
        .await
        .expect("GET /api/sessions")
        .json()
        .await
        .expect("session listing json");
    let child = listing["sessions"]
        .as_array()
        .and_then(|rows| rows.iter().find(|row| row["sessionId"] == child_id));
    assert_eq!(
        persisted_child["parent_session_id"], sid,
        "actual child summary lost parent metadata; node roster: {listing}"
    );
    let child = child
        .unwrap_or_else(|| panic!("persisted child {child_id} absent from node roster: {listing}"));
    assert_eq!(child["parentSessionId"], sid, "child row: {child}");
    assert_eq!(child["sessionKind"], persisted_child["session_kind"]);
    session.close().await;
}
