//! Agent actor and keeper against a fake `distill agent --leader stdio` (a Python 3 script).

#![cfg(unix)]

use std::path::PathBuf;
use std::time::Duration;

use distill_node::agent::{AgentConfig, spawn};
use distill_node::keeper::{BoardEvent, Keeper};
use serde_json::json;
use tokio::time::{Instant, timeout};

const FAKE_AGENT: &str = r#"
import json, sys

def send(o):
    sys.stdout.write(json.dumps(o) + "\n")
    sys.stdout.flush()

def notif(method, params):
    send({"jsonrpc": "2.0", "method": method, "params": params})

def ok(i, result):
    send({"jsonrpc": "2.0", "id": i, "result": result})

def pending(tool_call_id):
    return {"sessionId": "s1", "update": {"kind": "permission", "sessionUpdate": "pending_interaction", "tool_call_id": tool_call_id}}

for line in sys.stdin:
    m = json.loads(line)
    if "method" not in m:
        continue
    i, meth, p = m.get("id"), m["method"], m.get("params", {})
    if meth == "initialize":
        ok(i, {"authMethods": [{"id": "xai.api_key"}]})
    elif meth == "authenticate":
        ok(i, {})
    elif meth == "_x.ai/sessions/list":
        ok(i, {"sessions": [{"sessionId": "s1", "cwd": "/tmp", "activity": "idle"}]})
    elif meth == "session/load":
        ok(i, {})
        notif("_x.ai/session_notification", pending("t1"))
        notif("_x.ai/session_notification", pending("t1"))
        notif("_x.ai/session_notification", {"sessionId": "s1", "update": {"sessionUpdate": "interaction_resolved", "tool_call_id": "t1"}})
    elif meth == "wrap":
        ok(i, {})
        notif("_x.ai/session_notification", {"method": "x.ai/session_notification", "params": {"sessionId": "s1", "update": {"kind": "permission", "sessionUpdate": "pending_interaction", "tool_call_id": "t2"}}})
    elif meth == "echo":
        ok(i, p)
    elif meth == "boom":
        send({"jsonrpc": "2.0", "id": i, "error": {"code": -32000, "message": "nope"}})
    elif meth == "die":
        sys.exit(3)
    # "hang" and unknown methods get no reply
"#;

const WAIT: Duration = Duration::from_secs(10);

/// Writes the fake agent into a tempdir; the dir must outlive the child.
fn fake_config() -> (tempfile::TempDir, AgentConfig) {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("fake_agent.py");
    std::fs::write(&script, FAKE_AGENT).unwrap();
    let cfg = AgentConfig {
        distill_bin: PathBuf::from("python3"),
        args: vec![script.into_os_string()],
        extra_env: Vec::new(),
    };
    (dir, cfg)
}

#[tokio::test]
async fn concurrent_requests_correlate_by_id() {
    let (_dir, cfg) = fake_config();
    let (handle, _rx) = spawn(&cfg).await.unwrap();
    let (a, b) = tokio::join!(
        handle.request("echo", json!({"n": 1})),
        handle.request("echo", json!({"n": 2})),
    );
    assert_eq!(a.unwrap(), json!({"n": 1}));
    assert_eq!(b.unwrap(), json!({"n": 2}));
}

#[tokio::test]
async fn error_response_is_err() {
    let (_dir, cfg) = fake_config();
    let (handle, _rx) = spawn(&cfg).await.unwrap();
    let err = handle.request("boom", json!({})).await.unwrap_err();
    assert!(err.to_string().contains("nope"), "{err}");
}

#[tokio::test]
async fn notifications_reach_receiver() {
    let (_dir, cfg) = fake_config();
    let (handle, mut rx) = spawn(&cfg).await.unwrap();
    handle
        .request(
            "session/load",
            json!({"sessionId": "s1", "cwd": "/tmp", "mcpServers": []}),
        )
        .await
        .unwrap();
    let msg = timeout(WAIT, rx.recv()).await.unwrap().unwrap();
    assert_eq!(msg["method"], "_x.ai/session_notification");
    assert_eq!(
        msg["params"]["update"]["sessionUpdate"],
        "pending_interaction"
    );
}

#[tokio::test]
async fn child_exit_closes_receiver_and_fails_pending() {
    let (_dir, cfg) = fake_config();
    let (handle, mut rx) = spawn(&cfg).await.unwrap();
    let pending = {
        let handle = handle.clone();
        tokio::spawn(async move { handle.request("hang", json!({})).await })
    };
    tokio::time::sleep(Duration::from_millis(200)).await;
    handle.notify("die", json!({})).unwrap();
    let res = timeout(WAIT, pending).await.unwrap().unwrap();
    assert!(res.is_err());
    assert!(timeout(WAIT, rx.recv()).await.unwrap().is_none());
}

#[tokio::test]
async fn keeper_refresh_and_cwd() {
    let (_dir, cfg) = fake_config();
    let keeper = Keeper::start(cfg).await.unwrap();
    assert_eq!(keeper.rows().len(), 1);
    assert_eq!(keeper.rows()[0]["sessionId"], "s1");
    assert_eq!(keeper.cwd_of("s1").unwrap().to_string_lossy(), "/tmp");
    assert!(keeper.cwd_of("nope").is_none());
    assert_eq!(keeper.refresh().await.unwrap().len(), 1);
}

#[tokio::test]
async fn attach_emits_needs_input_once_and_resolves() {
    let (_dir, cfg) = fake_config();
    let keeper = Keeper::start(cfg).await.unwrap();
    let mut board = keeper.subscribe();
    let mut needs = keeper.needs_input();

    keeper.attach("s1").await.unwrap();
    // Attach is remembered: second call sends nothing new.
    keeper.attach("s1").await.unwrap();

    let first = timeout(WAIT, needs.recv()).await.unwrap().unwrap();
    assert_eq!(first.session_id, "s1");
    assert_eq!(first.tool_call_id, "t1");
    assert_eq!(first.kind, "permission");
    // Duplicate pending_interaction for t1 must not produce a second event.
    assert!(
        timeout(Duration::from_millis(300), needs.recv())
            .await
            .is_err()
    );

    let mut saw_needs_input = false;
    let deadline = Instant::now() + WAIT;
    loop {
        let event = timeout(
            deadline.saturating_duration_since(Instant::now()),
            board.recv(),
        )
        .await
        .unwrap()
        .unwrap();
        let BoardEvent::Upsert(rows) = event else {
            continue;
        };
        let activity = rows[0]["activity"].as_str().unwrap_or_default().to_owned();
        if activity == "needs_input" {
            saw_needs_input = true;
        } else if activity == "working" {
            break;
        }
    }
    assert!(saw_needs_input);
    assert_eq!(keeper.rows()[0]["activity"], "working");
}

#[tokio::test]
async fn wrapped_notification_is_unwrapped() {
    let (_dir, cfg) = fake_config();
    let keeper = Keeper::start(cfg).await.unwrap();
    let mut needs = keeper.needs_input();
    keeper.attach("s1").await.unwrap();
    loop {
        if timeout(WAIT, needs.recv())
            .await
            .unwrap()
            .unwrap()
            .tool_call_id
            == "t1"
        {
            break;
        }
    }
    keeper.handle().request("wrap", json!({})).await.unwrap();
    let event = timeout(WAIT, needs.recv()).await.unwrap().unwrap();
    assert_eq!(event.tool_call_id, "t2");
    assert_eq!(event.session_id, "s1");
}

#[tokio::test]
async fn keeper_respawns_and_reattaches_after_child_dies() {
    let (_dir, cfg) = fake_config();
    let keeper = Keeper::start(cfg).await.unwrap();
    keeper.attach("s1").await.unwrap();
    keeper.handle().notify("die", json!({})).unwrap();

    let deadline = Instant::now() + WAIT;
    let fresh = loop {
        match keeper.handle().request("echo", json!({"ok": true})).await {
            Ok(v) => break v,
            Err(_) if Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Err(e) => panic!("keeper did not respawn: {e}"),
        }
    };
    assert_eq!(fresh, json!({"ok": true}));
    assert_eq!(keeper.rows().len(), 1);
}
