//! Trusted router end to end over loopback: REST, board and session WebSockets, guard, static web app.
//! The agent is the same Python fake as tests/agent.rs, extended with `session/new` and a permission turn.

#![cfg(unix)]

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use distill_node::agent::AgentConfig;
use distill_node::guard::GuardConfig;
use distill_node::keeper::Keeper;
use distill_node::push::Push;
use distill_node::sep::{ClientMsg, PlanReplyOutcome, QuestionReplyOutcome};
use distill_node::trusted_router;
use futures_util::{SinkExt as _, StreamExt as _};
use serde_json::{Value, json};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;
use tokio_tungstenite::tungstenite::handshake::client::Request as WsRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::{self, Message};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

const OWNER: &str = "me@example.com";
const ORIGIN: &str = "https://node.example.com";
const WAIT: Duration = Duration::from_secs(10);

const FAKE_AGENT: &str = r###"
import json, os, sys

LOG = os.environ["FAKE_LOG"]
SESSIONS = [{"sessionId": "s1", "cwd": "/tmp", "activity": "idle"}]

def log(o):
    with open(LOG, "a") as f:
        f.write(json.dumps(o) + "\n")

def send(o):
    sys.stdout.write(json.dumps(o) + "\n")
    sys.stdout.flush()

def notif(method, params):
    send({"jsonrpc": "2.0", "method": method, "params": params})

def ok(i, result):
    send({"jsonrpc": "2.0", "id": i, "result": result})

def pending(tool_call_id):
    return {"sessionId": "s1", "update": {"kind": "permission", "sessionUpdate": "pending_interaction", "tool_call_id": tool_call_id}}

lines = iter(sys.stdin)
for line in lines:
    m = json.loads(line)
    log(m)
    if "method" not in m:
        continue
    i, meth, p = m.get("id"), m["method"], m.get("params", {})
    if meth == "initialize":
        ok(i, {"authMethods": [{"id": "xai.api_key"}]})
    elif meth == "authenticate":
        ok(i, {})
    elif meth == "_x.ai/sessions/list":
        ok(i, {"sessions": list(SESSIONS)})
    elif meth == "session/new":
        SESSIONS.append({"sessionId": "s2", "cwd": p.get("cwd", "/tmp"), "activity": "idle"})
        ok(i, {"sessionId": "s2"})
    elif meth == "session/load":
        ok(i, {})
        notif("_x.ai/session_notification", pending("t1"))
        notif("_x.ai/session_notification", {"sessionId": "s1", "update": {"sessionUpdate": "interaction_resolved", "tool_call_id": "t1"}})
    elif meth == "session/prompt":
        sid = p["sessionId"]
        prompt = " ".join(block.get("text", "") for block in p.get("prompt", []) if block.get("type") == "text").lower()
        if "question" in prompt:
            request_id, method, tool_call_id = 901, "_x.ai/ask_user_question", "q1"
            body = {"sessionId": sid, "toolCallId": tool_call_id, "mode": "default", "questions": [
                {"question": "Pick a color", "options": [{"label": "blue", "description": "Blue"}], "multiSelect": False},
                {"question": "Add a note", "options": [], "multiSelect": False}]}
        elif "plan" in prompt:
            request_id, method, tool_call_id = "plan-id", "_x.ai/exit_plan_mode", "p1"
            body = {"sessionId": sid, "toolCallId": tool_call_id, "planContent": "## Plan"}
        else:
            send({"jsonrpc": "2.0", "id": 900, "method": "session/request_permission", "params": {
                "sessionId": sid,
                "options": [{"optionId": "allow-once", "kind": "allow_once", "name": "Yes"}],
                "toolCall": {"toolCallId": "c1", "title": "Edit `a`"}}})
            for resp in lines:
                r = json.loads(resp)
                log(r)
                if r.get("id") == 900 and "method" not in r:
                    break
            notif("_x.ai/session_notification", {"sessionId": sid, "update": {"sessionUpdate": "turn_completed", "stop_reason": "end_turn"}})
            ok(i, {"stopReason": "end_turn"})
            continue
        send({"jsonrpc": "2.0", "id": request_id, "method": method, "params": {"method": method[1:], "params": body}})
        for resp in lines:
            r = json.loads(resp)
            log(r)
            if r.get("id") == request_id and "method" not in r:
                break
        notif("_x.ai/session_notification", {"sessionId": sid, "update": {"sessionUpdate": "interaction_resolved", "tool_call_id": tool_call_id}})
        for resp in lines:
            r = json.loads(resp)
            log(r)
            if r.get("method") == "session/cancel":
                break
        notif("_x.ai/session_notification", {"sessionId": sid, "update": {"sessionUpdate": "turn_completed", "stop_reason": "end_turn"}})
        ok(i, {"stopReason": "end_turn"})
"###;

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

struct Fixture {
    base: String,
    addr: SocketAddr,
    log: PathBuf,
    cwd: PathBuf,
    web: PathBuf,
    _dir: tempfile::TempDir,
}

/// Starts the trusted router on loopback with the fake agent, a temp web dir and a temp cwd.
async fn start() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("fake_agent.py");
    std::fs::write(&script, FAKE_AGENT).unwrap();
    let log = dir.path().join("fake.log");
    let cwd = dir.path().join("cwd");
    std::fs::create_dir(&cwd).unwrap();
    let web = dir.path().join("web");
    std::fs::create_dir(&web).unwrap();
    std::fs::write(web.join("index.html"), "<html>app</html>").unwrap();
    std::fs::write(dir.path().join("secret.txt"), "secret").unwrap();

    let agent = AgentConfig {
        distill_bin: PathBuf::from("python3"),
        args: vec![script.into_os_string()],
        extra_env: vec![("FAKE_LOG".into(), log.clone().into_os_string())],
    };
    let keeper = Keeper::start(agent.clone()).await.unwrap();
    let push = Arc::new(Push::load_or_create(&dir.path().join("state")).unwrap());
    let guard = Arc::new(GuardConfig {
        owner: OWNER.to_owned(),
        allowed_origins: vec![ORIGIN.to_owned()],
    });
    let app = trusted_router(keeper, push, agent, guard, Some(web.clone()));

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
        web,
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

fn ws_request(addr: SocketAddr, path: &str, origin: bool) -> WsRequest {
    let mut req = format!("ws://{addr}{path}").into_client_request().unwrap();
    req.headers_mut()
        .insert("Tailscale-User-Login", HeaderValue::from_static(OWNER));
    if origin {
        req.headers_mut()
            .insert("Origin", HeaderValue::from_static(ORIGIN));
    }
    req
}

async fn connect(addr: SocketAddr, path: &str) -> Ws {
    let (ws, _) = timeout(WAIT, connect_async(ws_request(addr, path, true)))
        .await
        .unwrap()
        .unwrap();
    ws
}

async fn next_json(ws: &mut Ws) -> Value {
    loop {
        let msg = timeout(WAIT, ws.next()).await.unwrap().unwrap().unwrap();
        if let Message::Text(text) = msg {
            return serde_json::from_str(&text).unwrap();
        }
    }
}

/// Skips frames until one with `"t": t` arrives.
async fn until(ws: &mut Ws, t: &str) -> Value {
    loop {
        let frame = next_json(ws).await;
        if frame["t"] == t {
            return frame;
        }
    }
}

async fn send_json(ws: &mut Ws, value: Value) {
    ws.send(Message::Text(value.to_string().into()))
        .await
        .unwrap();
}

async fn send_client_msg(ws: &mut Ws, msg: &ClientMsg) {
    send_json(ws, serde_json::to_value(msg).unwrap()).await;
}

fn log_lines(path: &Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

#[tokio::test]
async fn create_session_always_approve_sets_yolo_meta() {
    let f = start().await;
    let res = client()
        .post(format!("{}/api/sessions", f.base))
        .header("Tailscale-User-Login", OWNER)
        .json(&json!({"cwd": f.cwd, "mode": "always-approve"}))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body: Value = res.json().await.unwrap();
    assert_eq!(body, json!({"sessionId": "s2"}));

    let new = log_lines(&f.log)
        .into_iter()
        .find(|m| m["method"] == "session/new")
        .expect("session/new reached the agent");
    assert_eq!(new["params"]["_meta"]["yoloMode"], true);
    assert_eq!(new["params"]["cwd"], f.cwd.to_string_lossy().as_ref());
}

#[tokio::test]
async fn create_session_rejects_relative_cwd_and_worktree() {
    let f = start().await;
    let client = client();
    let relative = client
        .post(format!("{}/api/sessions", f.base))
        .header("Tailscale-User-Login", OWNER)
        .json(&json!({"cwd": "relative/dir"}))
        .send()
        .await
        .unwrap();
    assert_eq!(relative.status(), 400);

    let worktree = client
        .post(format!("{}/api/sessions", f.base))
        .header("Tailscale-User-Login", OWNER)
        .json(&json!({"cwd": f.cwd, "worktree": {"name": "wt"}}))
        .send()
        .await
        .unwrap();
    assert_eq!(worktree.status(), 501);
}

#[tokio::test]
async fn list_requires_owner() {
    let f = start().await;
    let client = client();
    let anon = client
        .get(format!("{}/api/sessions", f.base))
        .send()
        .await
        .unwrap();
    assert_eq!(anon.status(), 403);

    let ok: Value = client
        .get(format!("{}/api/sessions", f.base))
        .header("Tailscale-User-Login", OWNER)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(ok["sessions"][0]["sessionId"], "s1");
}

#[tokio::test]
async fn board_sends_upsert_first() {
    let f = start().await;
    let mut ws = connect(f.addr, "/ws/board").await;
    let first = next_json(&mut ws).await;
    assert_eq!(first["t"], "board.upsert");
    assert_eq!(first["sessions"][0]["sessionId"], "s1");
}

#[tokio::test]
async fn websocket_without_origin_is_refused() {
    let f = start().await;
    let err = connect_async(ws_request(f.addr, "/ws/board", false))
        .await
        .unwrap_err();
    match err {
        tungstenite::Error::Http(res) => assert_eq!(res.status(), 403),
        other => panic!("expected HTTP 403, got {other}"),
    }
}

#[tokio::test]
async fn session_replays_then_prompt_permission_turn() {
    let f = start().await;
    let mut ws = connect(f.addr, "/ws/session/s1").await;
    until(&mut ws, "replay.end").await;

    send_json(&mut ws, json!({"t": "prompt", "text": "hi"})).await;
    let req = until(&mut ws, "permission.request").await;
    assert_eq!(req["id"], 900);
    assert_eq!(req["tool_call_id"], "c1");

    send_json(
        &mut ws,
        json!({"t": "permission", "id": 900, "option_id": "allow-once"}),
    )
    .await;
    until(&mut ws, "turn.end").await;

    let answered = log_lines(&f.log).into_iter().any(|m| {
        m["id"] == 900
            && m.get("method").is_none()
            && m["result"]["outcome"]["optionId"] == "allow-once"
    });
    assert!(answered, "permission response reached the agent");
}

#[tokio::test]
async fn session_question_reply_validates_and_resolves_request() {
    timeout(WAIT, async {
        let f = start().await;
        let mut ws = connect(f.addr, "/ws/session/s1").await;
        until(&mut ws, "replay.end").await;
        send_client_msg(
            &mut ws,
            &ClientMsg::Prompt {
                text: "question flow".into(),
                images: vec![],
            },
        )
        .await;

        let req = until(&mut ws, "question.request").await;
        assert_eq!(req["id"], 901);
        assert_eq!(req["session_id"], "s1");
        assert_eq!(req["tool_call_id"], "q1");
        assert_eq!(req["questions"][0]["question"], "Pick a color");
        assert_eq!(req["questions"][1]["question"], "Add a note");

        let answers = std::collections::BTreeMap::from([
            ("Pick a color".into(), vec!["blue".into()]),
            ("Add a note".into(), vec!["free text".into()]),
        ]);
        let answer = ClientMsg::QuestionReply {
            id: json!(901),
            session_id: "s1".into(),
            tool_call_id: "q1".into(),
            outcome: QuestionReplyOutcome::Accepted,
            answers: Some(answers),
        };
        send_client_msg(
            &mut ws,
            &ClientMsg::QuestionReply {
                id: json!(902),
                session_id: "s1".into(),
                tool_call_id: "q1".into(),
                outcome: QuestionReplyOutcome::Accepted,
                answers: Some(std::collections::BTreeMap::new()),
            },
        )
        .await;
        assert_eq!(
            until(&mut ws, "error").await["message"],
            "unknown or stale interaction request id"
        );
        send_client_msg(
            &mut ws,
            &ClientMsg::PlanReply {
                id: json!(901),
                session_id: "s1".into(),
                tool_call_id: "q1".into(),
                outcome: PlanReplyOutcome::Approved,
                feedback: None,
            },
        )
        .await;
        assert_eq!(
            until(&mut ws, "error").await["message"],
            "interaction request kind does not match reply"
        );

        send_client_msg(&mut ws, &answer).await;
        assert_eq!(
            until(&mut ws, "permission.resolved").await["tool_call_id"],
            "q1"
        );
        send_client_msg(&mut ws, &answer).await;
        assert_eq!(
            until(&mut ws, "error").await["message"],
            "unknown or stale interaction request id"
        );
        send_client_msg(&mut ws, &ClientMsg::Cancel).await;
        until(&mut ws, "turn.end").await;

        let response = log_lines(&f.log)
            .into_iter()
            .find(|m| m["id"] == 901 && m.get("method").is_none())
            .expect("accepted answer reached the agent");
        assert_eq!(
            response["result"],
            json!({
                "outcome": "accepted",
                "answers": {"Pick a color": ["blue"], "Add a note": ["free text"]}
            })
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn session_plan_reply_validates_and_resolves_request() {
    timeout(WAIT, async {
        let f = start().await;
        let mut ws = connect(f.addr, "/ws/session/s1").await;
        until(&mut ws, "replay.end").await;
        send_client_msg(
            &mut ws,
            &ClientMsg::Prompt {
                text: "plan flow".into(),
                images: vec![],
            },
        )
        .await;

        let req = until(&mut ws, "plan.request").await;
        assert_eq!(req["id"], "plan-id");
        assert_eq!(req["session_id"], "s1");
        assert_eq!(req["tool_call_id"], "p1");
        assert_eq!(req["plan_content"], "## Plan");

        send_client_msg(
            &mut ws,
            &ClientMsg::Permission {
                id: json!("plan-id"),
                option_id: "allow-once".into(),
            },
        )
        .await;
        assert_eq!(
            until(&mut ws, "error").await["message"],
            "unknown permission request or option"
        );

        let answer = ClientMsg::PlanReply {
            id: json!("plan-id"),
            session_id: "s1".into(),
            tool_call_id: "p1".into(),
            outcome: PlanReplyOutcome::Approved,
            feedback: Some("Looks good".into()),
        };
        send_client_msg(
            &mut ws,
            &ClientMsg::PlanReply {
                id: json!("missing-id"),
                session_id: "s1".into(),
                tool_call_id: "p1".into(),
                outcome: PlanReplyOutcome::Approved,
                feedback: None,
            },
        )
        .await;
        assert_eq!(
            until(&mut ws, "error").await["message"],
            "unknown or stale interaction request id"
        );
        send_client_msg(
            &mut ws,
            &ClientMsg::QuestionReply {
                id: json!("plan-id"),
                session_id: "s1".into(),
                tool_call_id: "p1".into(),
                outcome: QuestionReplyOutcome::Accepted,
                answers: Some(std::collections::BTreeMap::from([(
                    "question".into(),
                    vec!["answer".into()],
                )])),
            },
        )
        .await;
        assert_eq!(
            until(&mut ws, "error").await["message"],
            "interaction request kind does not match reply"
        );

        send_client_msg(&mut ws, &answer).await;
        assert_eq!(
            until(&mut ws, "permission.resolved").await["tool_call_id"],
            "p1"
        );
        send_client_msg(&mut ws, &answer).await;
        assert_eq!(
            until(&mut ws, "error").await["message"],
            "unknown or stale interaction request id"
        );
        send_client_msg(&mut ws, &ClientMsg::Cancel).await;
        until(&mut ws, "turn.end").await;

        let response = log_lines(&f.log)
            .into_iter()
            .find(|m| m["id"] == "plan-id" && m.get("method").is_none())
            .expect("approved plan reply reached the agent");
        assert_eq!(
            response["result"],
            json!({"outcome": "approved", "feedback": "Looks good"})
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn session_unknown_id_reports_error() {
    let f = start().await;
    let mut ws = connect(f.addr, "/ws/session/nope").await;
    let frame = next_json(&mut ws).await;
    assert_eq!(frame, json!({"t": "error", "message": "unknown session"}));
}

#[tokio::test]
async fn static_web_app_and_spa_fallback() {
    let f = start().await;
    let client = client();
    let root = client.get(format!("{}/", f.base)).send().await.unwrap();
    assert_eq!(root.status(), 200);
    assert_eq!(root.text().await.unwrap(), "<html>app</html>");

    let deep = client
        .get(format!("{}/s/abc", f.base))
        .send()
        .await
        .unwrap();
    assert_eq!(deep.status(), 200);
    assert_eq!(deep.text().await.unwrap(), "<html>app</html>");

    let api = client
        .get(format!("{}/api/nope", f.base))
        .send()
        .await
        .unwrap();
    assert_eq!(api.status(), 404);

    // Encoded `..` survives client-side URL normalization and must not reach the secret.
    let traversal = client
        .get(format!("{}/%2e%2e%2fsecret.txt", f.base))
        .send()
        .await
        .unwrap();
    assert_eq!(traversal.status(), 404);

    let plain = client
        .get(format!("{}/../secret.txt", f.base))
        .send()
        .await
        .unwrap();
    assert_ne!(plain.text().await.unwrap(), "secret");
    assert!(f.web.join("index.html").exists());
    assert!(!f.cwd.join("secret.txt").exists());
}
