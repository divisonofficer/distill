use std::path::Path;
use std::time::Duration;

use distill_test_support::{Conversation, MockToolCall, ScriptViolation, StatusFailure, Tool};
use postudio_probe::{
    Harness, PidGuard, RawClient, activities_for, method_of, must, pid_alive, update_kind,
};
use serde_json::{Value, json};

fn edit_turn(n: usize, file: &Path, reply: &str) -> Conversation {
    Conversation::nth(n)
        .calls([MockToolCall::new(
            Tool::Edit,
            json!({"file_path": file, "old_string": "before", "new_string": "after"}),
        )])
        .reply(reply)
}

fn temp_file(h: &Harness, name: &str) -> std::path::PathBuf {
    let path = h.cwd.join(name);
    std::fs::write(&path, "before\n").expect("write temp file");
    path
}

async fn client(h: &Harness) -> RawClient {
    let mut c = h.spawn_client();
    must("initialize", c.initialize().await);
    c
}

/// Method plus `sessionUpdate` kind for each message, for observation output.
fn labels(msgs: &[Value]) -> Vec<String> {
    msgs.iter()
        .map(|m| match (method_of(m), update_kind(m)) {
            (Some(method), Some(kind)) => format!("{method}[{kind}]"),
            (Some(method), None) => method.to_owned(),
            _ => "<response>".to_owned(),
        })
        .collect()
}

/// `params` of a `x.ai/yolo_mode_changed` message, in direct or leader-wrapped form.
fn echo_params(m: &Value) -> &Value {
    let wrapped = m.pointer("/params/method").is_some();
    let inner = if wrapped {
        m.pointer("/params/params")
    } else {
        m.get("params")
    };
    inner.unwrap_or(&Value::Null)
}

/// Whether `m` is `x.ai/yolo_mode_changed` for `sid`, with or without the leading `_` and in direct or wrapped form.
fn is_mode_echo_for(m: &Value, sid: &str) -> bool {
    let direct = method_of(m).is_some_and(|mt| mt.trim_start_matches('_') == "x.ai/yolo_mode_changed");
    let wrapped = m
        .pointer("/params/method")
        .and_then(Value::as_str)
        .is_some_and(|mt| mt.trim_start_matches('_') == "x.ai/yolo_mode_changed");
    (direct || wrapped) && echo_params(m).get("sessionId").and_then(Value::as_str) == Some(sid)
}

/// Read incoming messages until a mode echo for `sid` arrives or `dur` passes. Returns whether it arrived.
async fn wait_for_mode_echo(c: &mut RawClient, sid: &str, dur: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + dur;
    loop {
        if c.incoming().iter().any(|m| is_mode_echo_for(m, sid)) {
            return true;
        }
        let now = tokio::time::Instant::now();
        if now >= deadline {
            return false;
        }
        c.pump_for((deadline - now).min(Duration::from_millis(200))).await;
    }
}

fn is_update(m: &Value) -> bool {
    method_of(m) == Some("session/update")
}

#[tokio::test]
#[ignore = "needs the distill binary; set GROK_BINARY"]
async fn l1_two_clients_share_session() {
    let h = Harness::start().await;
    let file = temp_file(&h, "a.txt");
    h.server.set_conversations([edit_turn(1, &file, "L1_DONE")]);
    let mut a = client(&h).await;
    let sid = must("A session/new", a.new_session(&h.cwd).await);
    let mut b = client(&h).await;
    let loaded = must("B session/load", b.load_session(&sid, &h.cwd).await);
    eprintln!("B session/load result keys: {:?}", loaded.as_object().map(|o| o.keys().collect::<Vec<_>>()));
    a.settle().await;
    b.settle().await;

    let prompt_id = a.send_prompt(&sid, "go").await;
    let perm_a = must("A permission", a.wait_permission(&sid, Duration::from_secs(30)).await);
    let perm_b = must("B permission", b.wait_permission(&sid, Duration::from_secs(30)).await);
    eprintln!("permission request A: {perm_a}\npermission request B: {perm_b}");
    must("B allow", b.answer_permission(&perm_b, "allow_once").await);
    let resp = must("A prompt response", a.await_response(prompt_id, Duration::from_secs(60)).await);
    eprintln!("A prompt response: {resp}");
    assert!(resp.get("error").is_none(), "prompt errored: {resp}");
    a.pump_for(Duration::from_secs(1)).await;
    b.pump_for(Duration::from_secs(1)).await;

    let after = std::fs::read_to_string(&file).unwrap();
    assert_eq!(after, "after\n", "tool did not run after B allowed");
    let a_updates = a.incoming().iter().filter(|m| is_update(m)).count();
    let b_updates = b.incoming().iter().filter(|m| is_update(m)).count();
    assert!(a_updates > 0, "A got no session/update: {}", a.summary());
    assert!(b_updates > 0, "B got no session/update: {}", b.summary());
    let reply_seen = |c: &RawClient| c.incoming().iter().any(|m| m.to_string().contains("L1_DONE"));
    assert!(reply_seen(&a) && reply_seen(&b), "scripted reply not seen by both");
    let a_labels = labels(a.incoming());
    let from = a_labels.iter().position(|l| l == "session/request_permission").unwrap_or(0);
    eprintln!("OBSERVE A incoming after permission request: {:?}", &a_labels[from..]);
    eprintln!("OBSERVE B incoming (all): {:?}", labels(b.incoming()));
    a.close().await;
    b.close().await;
}

#[tokio::test]
#[ignore = "needs the distill binary; set GROK_BINARY"]
async fn l1b_late_joiner_gets_pending_permission() {
    let h = Harness::start().await;
    let file = temp_file(&h, "a.txt");
    h.server.set_conversations([edit_turn(1, &file, "L1B_DONE")]);
    let mut a = client(&h).await;
    let sid = must("A session/new", a.new_session(&h.cwd).await);
    let prompt_id = a.send_prompt(&sid, "go").await;
    let perm_a = must("A permission", a.wait_permission(&sid, Duration::from_secs(30)).await);

    let mut c = client(&h).await;
    must("C session/load", c.load_session(&sid, &h.cwd).await);
    let perm_c = c.wait_permission(&sid, Duration::from_secs(15)).await;
    eprintln!("late joiner C incoming: {:?}", labels(c.incoming()));
    must("A allow", a.answer_permission(&perm_a, "allow_once").await);
    let resp = must("A prompt response", a.await_response(prompt_id, Duration::from_secs(60)).await);
    assert!(resp.get("error").is_none(), "prompt errored: {resp}");
    let perm_c = perm_c.unwrap_or_else(|e| panic!("late joiner never got pending request_permission: {e}"));
    eprintln!("late joiner C permission: {perm_c}");
    a.close().await;
    c.close().await;
}

/// Script and run a turn that starts a background `sleep`; returns the session id and the sleep pid.
async fn run_background_turn(h: &Harness, a: &mut RawClient) -> (String, u32) {
    let pid_file = h.sandbox.temp_dir().join("bg.pid");
    let command = format!("echo $$ > {}; exec sleep 600", pid_file.display());
    h.server.set_conversations([Conversation::nth(1)
        .calls([MockToolCall::new(
            Tool::Shell,
            json!({"command": command, "description": "start background sleep", "is_background": true}),
        )])
        .reply("L2_DONE")]);
    let sid = must("A session/new", a.new_session(&h.cwd).await);
    let prompt_id = a.send_prompt(&sid, "go").await;
    let (resp, answered) = must("turn", a.finish_turn(prompt_id, "allow_once").await);
    eprintln!("permission requests answered: {answered}; prompt response: {resp}");
    assert!(resp.get("error").is_none(), "prompt errored: {resp}");
    let mut pid = None;
    for _ in 0..50 {
        pid = std::fs::read_to_string(&pid_file).ok().and_then(|s| s.trim().parse::<u32>().ok());
        if pid.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let pid = pid.unwrap_or_else(|| panic!("background command never wrote its pid: {}", a.summary()));
    (sid, pid)
}

/// Known failure on unpatched Distill: the last client detaching unloads the idle session and its
/// background processes die with it. This is the regression check for the fix (plan/research-base/P0-RESULTS.md, L2).
#[tokio::test]
#[ignore = "needs the distill binary; set GROK_BINARY"]
async fn l2_background_process_survives_disconnect() {
    let h = Harness::start().await;
    let mut a = client(&h).await;
    let (sid, pid) = run_background_turn(&h, &mut a).await;
    let _guard = PidGuard(pid);
    assert!(pid_alive(pid), "background pid {pid} not alive before disconnect");

    a.close().await;
    tokio::time::sleep(Duration::from_secs(8)).await;
    assert!(pid_alive(pid), "background pid {pid} died after all clients disconnected");

    let mut n = client(&h).await;
    let loaded = n.load_session(&sid, &h.cwd).await;
    eprintln!("session/load after disconnect: {loaded:?}");
    must("session/load after disconnect", loaded);
    n.close().await;
}

#[tokio::test]
#[ignore = "needs the distill binary; set GROK_BINARY"]
async fn l2b_background_process_with_one_client_still_attached() {
    let h = Harness::start().await;
    let mut a = client(&h).await;
    let (sid, pid) = run_background_turn(&h, &mut a).await;
    let _guard = PidGuard(pid);
    assert!(pid_alive(pid), "background pid {pid} not alive before disconnect");

    let mut b = client(&h).await;
    must("B session/load", b.load_session(&sid, &h.cwd).await);
    a.close().await;
    tokio::time::sleep(Duration::from_secs(8)).await;
    let alive_with_b = pid_alive(pid);
    eprintln!("OBSERVE l2b: pid {pid} alive after A left, B attached: {alive_with_b}");
    assert!(alive_with_b, "background pid {pid} died although client B stayed attached");

    b.close().await;
    tokio::time::sleep(Duration::from_secs(8)).await;
    eprintln!("OBSERVE l2b: pid {pid} alive after B also left: {}", pid_alive(pid));
}

#[tokio::test]
#[ignore = "needs the distill binary; set GROK_BINARY"]
async fn l3_leader_survives_zero_clients() {
    let h = Harness::start().await;
    h.server.set_conversations([Conversation::nth(1).reply("L3_DONE")]);
    let leader_pid = h.leader.leader_pid().expect("leader pid");
    let mut a = client(&h).await;
    let sid = must("A session/new", a.new_session(&h.cwd).await);
    let prompt_id = a.send_prompt(&sid, "go").await;
    must("A prompt response", a.await_response(prompt_id, Duration::from_secs(60)).await);
    a.close().await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert!(pid_alive(leader_pid), "leader pid {leader_pid} died with zero clients");

    let mut n = client(&h).await;
    let mut listed = None;
    for method in ["_x.ai/sessions/list", "x.ai/sessions/list", "session/list"] {
        match n.request(method, json!({})).await {
            Ok(result) => {
                eprintln!("list via {method}: {result}");
                listed = Some((method, result));
                break;
            }
            Err(e) => eprintln!("list via {method} failed: {e}"),
        }
    }
    let (method, result) = listed.expect("no list method reachable from this client");
    eprintln!("session list method used: {method}");
    assert!(result.to_string().contains(&sid), "session {sid} missing from {method} result: {result}");
    n.close().await;
}

#[tokio::test]
#[ignore = "needs the distill binary; set GROK_BINARY"]
async fn l4_session_scoped_mode_change() {
    let h = Harness::start().await;
    let f1 = temp_file(&h, "s1.txt");
    let f2 = temp_file(&h, "s2.txt");
    let f3 = temp_file(&h, "s3.txt");
    h.server.set_conversations([
        edit_turn(1, &f1, "S1_DONE"),
        edit_turn(2, &f2, "S2_DONE"),
        edit_turn(3, &f3, "S3_DONE"),
    ]);
    let mut a = client(&h).await;
    let s1 = must("S1", a.new_session(&h.cwd).await);
    let s2 = must("S2", a.new_session(&h.cwd).await);
    let mut b = client(&h).await;
    must("B load S1", b.load_session(&s1, &h.cwd).await);
    b.settle().await;
    a.settle().await;

    a.notify("_x.ai/yolo_mode_changed", json!({"yolo_mode": true, "sessionId": s1})).await;
    let echo_seen = wait_for_mode_echo(&mut b, &s1, Duration::from_secs(10)).await;
    assert!(
        echo_seen,
        "B (attached to S1) did not receive x.ai/yolo_mode_changed for S1 within 10 s; saw {:?}",
        labels(b.incoming())
    );
    let echo = b.incoming().iter().find(|m| is_mode_echo_for(m, &s1)).cloned().expect("echo present");
    assert_eq!(echo_params(&echo).get("yolo_mode"), Some(&json!(true)), "echo carries new mode: {echo}");
    a.pump_for(Duration::from_millis(500)).await;
    eprintln!("OBSERVE B after mode change: {:?}", labels(b.incoming()));
    eprintln!("OBSERVE A after mode change: {:?}", labels(a.incoming()));

    let p1 = a.send_prompt(&s1, "go").await;
    let (r1, asked1) = must("S1 turn", a.finish_turn(p1, "allow_once").await);
    eprintln!("S1: response {r1}; permission requests: {asked1}");
    let p2 = a.send_prompt(&s2, "go").await;
    let (r2, asked2) = must("S2 turn", a.finish_turn(p2, "allow_once").await);
    eprintln!("S2: response {r2}; permission requests: {asked2}");
    eprintln!("files: s1={:?} s2={:?}", std::fs::read_to_string(&f1), std::fs::read_to_string(&f2));
    let s3 = must("S3", a.new_session(&h.cwd).await);
    let p3 = a.send_prompt(&s3, "go").await;
    let (r3, asked3) = must("S3 turn", a.finish_turn(p3, "allow_once").await);
    eprintln!("S3: response {r3}; permission requests: {asked3}");
    eprintln!("OBSERVE l4 permission counts: S1={asked1} S2={asked2} S3={asked3}");
    assert_eq!(asked1, 0, "S1 (yolo) still sent session/request_permission");
    assert_eq!(std::fs::read_to_string(&f1).unwrap(), "after\n", "S1 tool did not run");
    assert!(asked2 >= 1, "S2 did not send session/request_permission");
    assert!(
        asked3 >= 1,
        "S3 sent no session/request_permission: session-scoped mode change leaked into a connection default"
    );
    a.close().await;
    b.close().await;
}

#[tokio::test]
#[ignore = "needs the distill binary; set GROK_BINARY"]
async fn l7_roster_broadcast_on_needs_input() {
    let h = Harness::start().await;
    let file = temp_file(&h, "a.txt");
    h.server.set_conversations([edit_turn(1, &file, "L7_DONE")]);
    let mut d = client(&h).await;
    let mut a = client(&h).await;
    let sid = must("A session/new", a.new_session(&h.cwd).await);
    let prompt_id = a.send_prompt(&sid, "go").await;
    let perm = must("A permission", a.wait_permission(&sid, Duration::from_secs(30)).await);

    let roster_for_sid = |c: &RawClient| -> Vec<Value> {
        c.incoming()
            .iter()
            .filter(|m| method_of(m).is_some_and(|x| x.contains("sessions/changed")))
            .filter(|m| m.to_string().contains(&sid))
            .cloned()
            .collect()
    };
    let first = d
        .wait_for("roster notification for session", Duration::from_secs(15), |m| {
            method_of(m).is_some_and(|x| x.contains("sessions/changed")) && m.to_string().contains(&sid)
        })
        .await;
    d.pump_for(Duration::from_secs(3)).await;
    let listed = must("D _x.ai/sessions/list", d.request("_x.ai/sessions/list", json!({})).await);
    let entry = listed
        .pointer("/result/sessions")
        .or_else(|| listed.get("sessions"))
        .and_then(Value::as_array)
        .and_then(|rows| rows.iter().find(|r| r.get("sessionId").and_then(Value::as_str) == Some(sid.as_str())))
        .cloned();
    eprintln!("D sessions/list entry while pending: {entry:?}");
    let all = roster_for_sid(&d);
    for n in &all {
        eprintln!("D roster notification: {n}");
    }
    let acts: Vec<Vec<String>> = all.iter().map(|n| activities_for(n, &sid)).collect();
    eprintln!("D roster activities in order: {acts:?}");
    eprintln!("D all incoming: {:?}", labels(d.incoming()));
    must("A allow", a.answer_permission(&perm, "allow_once").await);
    must("A prompt response", a.await_response(prompt_id, Duration::from_secs(60)).await);
    must("D roster notification", first);
    let activity = entry.as_ref().and_then(|e| e.get("activity")).and_then(Value::as_str);
    assert_eq!(activity, Some("needs_input"), "list entry while permission pending: {entry:?}");
    d.close().await;
    a.close().await;
}

/// The agent reports the machine's host name in `initialize`; fixtures must not carry it.
fn scrub_hostname(v: &mut Value) {
    match v {
        Value::Object(map) => {
            if let Some(host) = map.get_mut("hostname") {
                *host = json!("host.local");
            }
            map.values_mut().for_each(scrub_hostname);
        }
        Value::Array(items) => items.iter_mut().for_each(scrub_hostname),
        _ => {}
    }
}

fn write_log(h: &Harness, name: &str, log: &[Value]) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../distill-node/tests/fixtures/acp");
    std::fs::create_dir_all(&dir).unwrap();
    let mut out = String::new();
    for entry in log {
        let mut entry = entry.clone();
        scrub_hostname(&mut entry);
        out.push_str(&h.sanitize(&entry.to_string()));
        out.push('\n');
    }
    std::fs::write(dir.join(format!("{name}.jsonl")), out).unwrap();
    eprintln!("wrote fixture {name}: {} lines", log.len());
}

#[tokio::test]
#[ignore = "needs the distill binary; set GROK_BINARY"]
async fn capture_fixtures() {
    let write = std::env::var("POSTUDIO_WRITE_FIXTURES").as_deref() == Ok("1");
    let h = Harness::start().await;
    let edit3 = temp_file(&h, "allow.txt");
    let edit4 = temp_file(&h, "deny.txt");
    let read2 = temp_file(&h, "read.txt");
    let png = h.cwd.join("tiny.png");
    let png_bytes = base64_png();
    std::fs::write(&png, png_bytes).unwrap();
    h.server.set_conversations([
        Conversation::nth(1).reply("Hello from the mock."),
        Conversation::nth(2)
            .calls([MockToolCall::new(Tool::Read, json!({"target_file": read2}))])
            .reply("Read done."),
        edit_turn(3, &edit3, "Edit allowed."),
        edit_turn(4, &edit4, "Edit denied."),
        Conversation::nth(5).refuse(StatusFailure::new(400).with_body("mock model failure")).reply("unreached"),
        Conversation::nth(6)
            .calls([MockToolCall::new(Tool::Read, json!({"target_file": png}))])
            .reply("Image done."),
    ]);
    let mut c = h.spawn_client();
    must("initialize", c.initialize().await);
    let mut start = 0;
    let mut sids = Vec::new();
    let mut saved: Vec<(&str, Vec<Value>)> = Vec::new();
    for (name, kind) in [
        ("text_reply", "allow_once"),
        ("tool_auto", "allow_once"),
        ("tool_permission_allow", "allow_once"),
        ("tool_permission_deny", "reject_once"),
        ("model_error", "allow_once"),
        ("image_output", "allow_once"),
    ] {
        let sid = must("session/new", c.new_session(&h.cwd).await);
        let p = c.send_prompt(&sid, "go").await;
        let (resp, asked) = must(name, c.finish_turn(p, kind).await);
        eprintln!("{name}: permission requests answered={asked}; prompt response={resp}");
        c.pump_for(Duration::from_millis(700)).await;
        let log = c.log();
        let slice = log[start..].to_vec();
        start = log.len();
        sids.push(sid);
        if name == "tool_permission_allow" {
            assert_eq!(asked, 1, "allow scenario expected one permission request");
            assert_eq!(std::fs::read_to_string(&edit3).unwrap(), "after\n");
        }
        if name == "tool_permission_deny" {
            assert_eq!(asked, 1, "deny scenario expected one permission request");
            assert_eq!(std::fs::read_to_string(&edit4).unwrap(), "before\n");
        }
        if name == "tool_auto" {
            assert_eq!(asked, 0, "read_file should not need approval");
        }
        saved.push((name, slice));
    }
    let image_ok = saved
        .iter()
        .find(|(n, _)| *n == "image_output")
        .is_some_and(|(_, log)| log.iter().any(|e| e.to_string().contains("\"type\":\"image\"")));
    eprintln!("image content block present: {image_ok}");

    let mut l = h.spawn_client();
    must("loader initialize", l.initialize().await);
    must("session/load replay", l.load_session(&sids[1], &h.cwd).await);
    l.pump_for(Duration::from_secs(2)).await;
    let replay = l.log();
    eprintln!("replay log labels: {:?}", labels(&replay.iter().filter_map(|e| e.get("msg").cloned()).collect::<Vec<_>>()));
    saved.push(("session_load_replay", replay));

    if write {
        for (name, log) in &saved {
            if *name == "image_output" && !image_ok {
                eprintln!("skipping image_output: no image content block");
                continue;
            }
            write_log(&h, name, log);
        }
    }
    c.close().await;
    l.close().await;
    let violations = h.server.finish_scripts();
    for v in &violations {
        eprintln!("script violation: {v}");
    }
    let mut unfinished: Vec<usize> = violations
        .iter()
        .filter_map(|v| match v {
            ScriptViolation::Unfinished { conversation, .. } => Some(*conversation),
            _ => None,
        })
        .collect();
    unfinished.sort_unstable();
    assert!(
        unfinished == [4, 5] && violations.len() == 2,
        "expected only conversations 4 and 5 unfinished, got {violations:?}"
    );
}

fn base64_png() -> Vec<u8> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD
        .decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==")
        .unwrap()
}
