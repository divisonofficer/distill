#![cfg(unix)]

use std::path::PathBuf;
use std::time::Duration;

use distill_node::agent::AgentConfig;
use distill_node::keeper::Keeper;
use serde_json::json;
use tokio::time::Instant;

const FAKE_AGENT: &str = r#"
import json, os, sys

def send(o):
    sys.stdout.write(json.dumps(o) + "\n")
    sys.stdout.flush()

def ok(i, result): send({"jsonrpc":"2.0", "id":i, "result":result})
for line in sys.stdin:
    m=json.loads(line)
    method=m.get("method")
    i=m.get("id")
    if method == "initialize": ok(i, {"authMethods":[{"id":"xai.api_key"}]})
    elif method == "authenticate": ok(i, {})
    elif method == "_x.ai/sessions/list":
        sessions = [
            {"sessionId":"s1", "cwd":"/tmp", "activity":"idle", "resident":True},
            {"sessionId":"history", "cwd":"/tmp", "activity":"dormant", "resident":False},
            {"sessionId":"working", "cwd":"/tmp", "activity":"working", "resident":False},
            {"sessionId":"needs", "cwd":"/tmp", "activity":"needs_input", "resident":False},
            {"sessionId":"stale", "cwd":"/missing", "activity":"idle", "resident":True}
        ]
        if m.get("params", {}).get("includeSubagents"):
            sessions.extend([
                {"sessionId":"child", "cwd":"/tmp", "activity":"dormant", "resident":False, "parentSessionId":"s1", "sessionKind":"subagent"},
                {"sessionId":"nested", "cwd":"/tmp", "activity":"dormant", "resident":False, "parentSessionId":"child", "sessionKind":"subagent_fork"}
            ])
        ok(i, {"sessions":sessions})
    elif method == "session/load":
        sid = m["params"]["sessionId"]
        with open(os.environ["KEEPER_LOAD_LOG"], "a+") as f:
            f.seek(0)
            prior = f.read().splitlines()
            f.seek(0, 2)
            f.write(sid + "\n")
        fail = sid == "stale" or (sid == "s1" and prior.count("s1") == 1)
        if fail:
            send({"jsonrpc":"2.0", "id":i, "error":{"code":-1,"message":"stale session"}})
        else:
            ok(i, {})
    elif method == "echo": ok(i, m.get("params", {}))
    elif method == "die": sys.exit(3)
"#;

fn fake_config() -> (tempfile::TempDir, AgentConfig) {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("fake_agent.py");
    let log = dir.path().join("loads");
    std::fs::write(&script, FAKE_AGENT).unwrap();
    let cfg = AgentConfig {
        distill_bin: PathBuf::from("python3"),
        args: vec![script.into_os_string()],
        extra_env: vec![("KEEPER_LOAD_LOG".into(), log.into_os_string())],
    };
    (dir, cfg)
}

#[tokio::test]
async fn startup_and_reconnect_restore_roster_sessions() {
    let (dir, cfg) = fake_config();
    let keeper = Keeper::start(cfg).await.unwrap();
    let child = keeper
        .rows()
        .into_iter()
        .find(|row| row["sessionId"] == "child")
        .unwrap();
    assert_eq!(child["parentSessionId"], "s1");
    assert_eq!(child["sessionKind"], "subagent");
    let nested = keeper
        .rows()
        .into_iter()
        .find(|row| row["sessionId"] == "nested")
        .unwrap();
    assert_eq!(nested["parentSessionId"], "child");
    assert_eq!(nested["sessionKind"], "subagent_fork");
    let log = dir.path().join("loads");
    let startup_loads = std::fs::read_to_string(&log).unwrap();
    assert_eq!(startup_loads.matches("s1\n").count(), 1);
    assert_eq!(startup_loads.matches("working\n").count(), 1);
    assert_eq!(startup_loads.matches("needs\n").count(), 1);
    assert!(!startup_loads.contains("history\n"));
    assert!(!startup_loads.contains("dormant\n"));
    assert!(startup_loads.contains("stale\n"));

    keeper.handle().notify("die", json!({})).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let loads = std::fs::read_to_string(&log).unwrap();
        if loads.matches("s1\n").count() >= 2 {
            break;
        }
        match keeper.handle().request("echo", json!({"ready":true})).await {
            Err(_) | Ok(_) if Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(100)).await
            }
            result => panic!("keeper did not reconnect: {result:?}"),
        }
    }

    // Reconnect attempt for s1 failed; explicit attach must issue a fresh load.
    keeper.attach("s1").await.unwrap();
    assert_eq!(
        std::fs::read_to_string(&log)
            .unwrap()
            .matches("s1\n")
            .count(),
        3
    );
}
