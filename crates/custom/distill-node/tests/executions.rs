#![cfg(unix)]

use distill_node::{
    agent::AgentConfig, guard::GuardConfig, keeper::Keeper, push::Push, trusted_router,
};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc};

const FAKE: &str = r#"
import json, os, sys
control=os.environ['CONTROL']; log=os.environ['LOG']; cwd=os.environ['CWD']
def send(id, result):
 print(json.dumps({'jsonrpc':'2.0','id':id,'result':result}),flush=True)
for line in sys.stdin:
 r=json.loads(line)
 if 'method' not in r: continue
 m=r['method']; p=r.get('params',{}); i=r.get('id')
 with open(log,'a') as f: f.write(json.dumps(r)+'\n')
 if m=='initialize': send(i,{'authMethods':[{'id':'xai.api_key'}]})
 elif m=='authenticate': send(i,{})
 elif m=='_x.ai/sessions/list': send(i,{'sessions':[{'sessionId':'s1','cwd':cwd,'activity':'idle'},{'sessionId':'s2','cwd':cwd,'activity':'idle'}]})
 elif m=='session/load': send(i,{})
 else:
  with open(control) as f: data=json.load(f)
  value=data.get(m,{'error':'unsupported','result':None})
  if value=='rpc-error': print(json.dumps({'jsonrpc':'2.0','id':i,'error':{'code':-32601,'message':'unsupported'}}),flush=True)
  else: send(i,{'result':value,'error':None})
"#;

struct Fixture {
    base: String,
    control: PathBuf,
    log: PathBuf,
    _dir: tempfile::TempDir,
}
impl Fixture {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("agent.py");
        std::fs::write(&script, FAKE).unwrap();
        let control = dir.path().join("control");
        let log = dir.path().join("log");
        std::fs::write(&control, "{}").unwrap();
        let agent = AgentConfig {
            distill_bin: "python3".into(),
            args: vec![script.into_os_string()],
            extra_env: vec![
                ("CONTROL".into(), control.clone().into_os_string()),
                ("LOG".into(), log.clone().into_os_string()),
                ("CWD".into(), dir.path().as_os_str().to_owned()),
            ],
        };
        let keeper = Keeper::start(agent.clone()).await.unwrap();
        let state = dir.path().join("state");
        let app = trusted_router(
            keeper,
            Arc::new(Push::load_or_create(&state).unwrap()),
            agent,
            Arc::new(GuardConfig {
                owner: "owner".into(),
                allowed_origins: vec![],
            }),
            None,
            distill_node::chat_store::Store::open(&state.join("chats")).unwrap(),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Self {
            base,
            control,
            log,
            _dir: dir,
        }
    }
    fn set(&self, v: Value) {
        std::fs::write(&self.control, v.to_string()).unwrap();
    }
    fn calls(&self, method: &str) -> Vec<Value> {
        std::fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .map(|s| serde_json::from_str::<Value>(s).unwrap())
            .filter(|v| v["method"] == method)
            .collect()
    }
    async fn request(&self, method: &str, path: &str, auth: bool) -> (u16, Value) {
        #[allow(clippy::disallowed_methods, reason = "localhost-only integration test")]
        let client = reqwest::Client::new();
        let mut r = client.request(method.parse().unwrap(), format!("{}{path}", self.base));
        if auth {
            r = r.header("Tailscale-User-Login", "owner");
        }
        if method == "POST" {
            r = r.header("content-type", "application/json").body("{}");
        }
        let r = r.send().await.unwrap();
        let status = r.status().as_u16();
        let body = r.json().await.unwrap_or(Value::Null);
        (status, body)
    }
}
fn agent(parent: &str, status: &str) -> Value {
    json!({"subagentId":"a","parentSessionId":parent,"childSessionId":"child","toolCallId":"tool-call-123","subagentType":"worker","description":"work","startedAtEpochMs":1000,"durationMs":50,"status":status})
}
fn task(completed: bool) -> Value {
    json!({"task_id":"t","command":"wrapped","display_command":null,"cwd":"/tmp","start_time":{"secs_since_epoch":10,"nanos_since_epoch":123000000},"end_time":null,"output":"hello","output_file":"/tmp/output","truncated":false,"exit_code":null,"signal":null,"completed":completed,"kind":"bash","output_total_bytes":5,"owner_session_id":"s1","is_backgrounded":true,"description":"real label"})
}
#[tokio::test]
async fn registry_routes_validate_ownership_and_outcomes() {
    let f = Fixture::new().await;
    f.set(json!({"_x.ai/subagent/list_running":{"subagents":[agent("s1","running")]},"_x.ai/subagent/get":{"snapshot":agent("s1","running")},"_x.ai/subagent/cancel":{"subagentId":"a","cancelled":true,"outcome":{"kind":"cancelled"}},"_x.ai/task/list":{"scopedKillSupported":true,"tasks":[task(false)]},"_x.ai/task/kill_scoped":{"taskId":"t","outcome":"killed"}}));
    assert_eq!(
        f.request("GET", "/api/sessions/s1/tasks", false).await.0,
        403
    );
    let (status, body) = f.request("GET", "/api/sessions/s1/subagents", true).await;
    assert_eq!(status, 200);
    assert_eq!(body["subagents"][0]["status"], "running");
    assert_eq!(body["subagents"][0]["toolCallId"], "tool-call-123");
    let snapshot = f
        .request("GET", "/api/sessions/s1/subagents/a", true)
        .await
        .1["snapshot"]
        .clone();
    assert_eq!(snapshot["parentSessionId"], "s1");
    assert_eq!(snapshot["toolCallId"], "tool-call-123");
    let (status, body) = f.request("GET", "/api/sessions/s1/tasks", true).await;
    assert_eq!(status, 200);
    assert_eq!(body["tasks"][0]["startedAtUnixMs"], 10123);
    assert_eq!(body["tasks"][0]["command"], "wrapped");
    assert_eq!(body["tasks"][0]["output"], "hello");
    assert_eq!(
        f.request("POST", "/api/sessions/s1/tasks/foreign/stop", true)
            .await
            .0,
        404
    );
    assert!(f.calls("_x.ai/task/kill_scoped").is_empty());
    assert_eq!(
        f.request("POST", "/api/sessions/s1/tasks/t/stop", true)
            .await
            .1,
        json!({"taskId":"t","outcome":"killed"})
    );
    assert_eq!(
        f.calls("_x.ai/task/kill_scoped")[0]["params"]["source"],
        "clientUi"
    );
    assert_eq!(
        f.request("POST", "/api/sessions/s1/subagents/a/cancel", true)
            .await
            .1["outcome"]["kind"],
        "cancelled"
    );
    f.set(json!({"_x.ai/subagent/get":{"snapshot":agent("s2","running")},"_x.ai/task/list":{"tasks":[task(true)]}}));
    assert_eq!(
        f.request("GET", "/api/sessions/s1/subagents/a", true)
            .await
            .0,
        404
    );
    assert_eq!(
        f.request("POST", "/api/sessions/s1/subagents/a/cancel", true)
            .await
            .0,
        404
    );
    assert_eq!(f.calls("_x.ai/subagent/cancel").len(), 1);
    assert_eq!(
        f.request("POST", "/api/sessions/s1/tasks/t/stop", true)
            .await
            .1,
        json!({"taskId":"t","outcome":"already_exited"})
    );
    assert_eq!(f.calls("_x.ai/task/kill_scoped").len(), 1);
    f.set(json!({"_x.ai/subagent/get":{"snapshot":agent("s1","completed")}}));
    let completed = f
        .request("GET", "/api/sessions/s1/subagents/a", true)
        .await
        .1["snapshot"]
        .clone();
    assert_eq!(completed["status"], "completed");
    assert_eq!(completed["toolCallId"], "tool-call-123");
    assert_eq!(
        f.request("POST", "/api/sessions/s1/subagents/a/cancel", true)
            .await
            .1,
        json!({"subagentId":"a","cancelled":false,"outcome":{"kind":"already_finished","status":"completed"}})
    );
    assert_eq!(f.calls("_x.ai/subagent/cancel").len(), 1);
}
#[tokio::test]
async fn list_subagents_requests_recent_history_and_preserves_statuses() {
    let f = Fixture::new().await;
    let rows = ["running", "completed", "failed", "cancelled"]
        .into_iter()
        .map(|status| agent("s1", status))
        .collect::<Vec<_>>();
    f.set(json!({"_x.ai/subagent/list_running":{"subagents":rows}}));

    let (status, body) = f.request("GET", "/api/sessions/s1/subagents", true).await;
    assert_eq!(status, 200);
    assert_eq!(
        body["subagents"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["status"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["running", "completed", "failed", "cancelled"]
    );
    assert_eq!(
        f.calls("_x.ai/subagent/list_running")[0]["params"]["includeCompleted"],
        true
    );

    f.set(json!({
        "_x.ai/subagent/list_running": {
            "subagents": [agent("other-session", "completed")]
        }
    }));
    assert_eq!(
        f.request("GET", "/api/sessions/s1/subagents", true).await.0,
        502
    );

    let mut malformed = agent("s1", "running");
    malformed["status"] = json!("unknown");
    f.set(json!({"_x.ai/subagent/list_running":{"subagents":[malformed]}}));
    assert_eq!(
        f.request("GET", "/api/sessions/s1/subagents", true).await.0,
        502
    );
}

#[tokio::test]
async fn subagent_call_id_is_optional_but_malformed_values_fail_closed() {
    let f = Fixture::new().await;
    let mut legacy = agent("s1", "running");
    legacy.as_object_mut().unwrap().remove("toolCallId");
    f.set(json!({
        "_x.ai/subagent/list_running": {"subagents": [legacy.clone()]},
        "_x.ai/subagent/get": {"snapshot": legacy},
    }));
    assert_eq!(
        f.request("GET", "/api/sessions/s1/subagents", true).await.0,
        200
    );
    assert_eq!(
        f.request("GET", "/api/sessions/s1/subagents/a", true)
            .await
            .0,
        200
    );

    let mut malformed = agent("s1", "running");
    malformed["toolCallId"] = json!("");
    f.set(json!({
        "_x.ai/subagent/list_running": {"subagents": [malformed.clone()]},
        "_x.ai/subagent/get": {"snapshot": malformed},
    }));
    assert_eq!(
        f.request("GET", "/api/sessions/s1/subagents", true).await.0,
        502
    );
    assert_eq!(
        f.request("GET", "/api/sessions/s1/subagents/a", true)
            .await
            .0,
        502
    );
}

#[tokio::test]
async fn upstream_outcomes_and_bounded_output() {
    let f = Fixture::new().await;
    let mut t = task(false);
    t["output"] = json!("é".repeat(600_000));
    f.set(json!({"_x.ai/task/list":{"scopedKillSupported":true,"tasks":[t]},"_x.ai/task/kill_scoped":{"taskId":"t","outcome":"already_exited"},"_x.ai/subagent/get":{"snapshot":agent("s1","running")},"_x.ai/subagent/cancel":{"subagentId":"a","cancelled":false,"outcome":{"kind":"not_found"}}}));
    let (status, body) = f.request("GET", "/api/sessions/s1/tasks", true).await;
    assert_eq!(status, 200);
    assert_eq!(body["tasks"][0]["truncated"], true);
    assert!(body["tasks"][0]["output"].as_str().unwrap().len() <= 1024 * 1024);
    assert_eq!(
        f.request("POST", "/api/sessions/s1/tasks/t/stop", true)
            .await
            .1["outcome"],
        "already_exited"
    );
    assert_eq!(
        f.request("POST", "/api/sessions/s1/subagents/a/cancel", true)
            .await
            .1["outcome"]["kind"],
        "not_found"
    );
    f.set(json!({"_x.ai/task/list":{"scopedKillSupported":true,"tasks":[task(false)]},"_x.ai/task/kill_scoped":{"taskId":"t","outcome":"not_found"},"_x.ai/subagent/get":{"snapshot":agent("s1","running")},"_x.ai/subagent/cancel":{"subagentId":"a","cancelled":false,"outcome":{"kind":"already_finished","status":"failed"}}}));
    assert_eq!(
        f.request("POST", "/api/sessions/s1/tasks/t/stop", true)
            .await
            .1["outcome"],
        "not_found"
    );
    assert_eq!(
        f.request("POST", "/api/sessions/s1/subagents/a/cancel", true)
            .await
            .1["outcome"]["status"],
        "failed"
    );
}

#[tokio::test]
async fn malformed_and_unsupported_fail_closed() {
    let f = Fixture::new().await;
    assert_eq!(
        f.request("GET", "/api/sessions/s1/tasks", true).await.0,
        502
    );
    for bad in [
        {
            let mut t = task(false);
            t["cwd"] = json!(null);
            t
        },
        {
            let mut t = task(false);
            t["start_time"]["nanos_since_epoch"] = json!(1000000000);
            t
        },
        {
            let mut t = task(false);
            t["start_time"]["secs_since_epoch"] = json!(-1);
            t
        },
    ] {
        f.set(json!({"_x.ai/task/list":{"tasks":[bad]}}));
        assert_eq!(
            f.request("GET", "/api/sessions/s1/tasks", true).await.0,
            502
        );
        assert_eq!(
            f.request("POST", "/api/sessions/s1/tasks/t/stop", true)
                .await
                .0,
            502
        );
    }
    f.set(json!({"_x.ai/task/list":{"scopedKillSupported":true,"tasks":[task(false)]},"_x.ai/task/kill_scoped":{"taskId":"foreign","outcome":"killed"},"_x.ai/subagent/get":{"snapshot":agent("s1","mystery")}}));
    assert_eq!(
        f.request("POST", "/api/sessions/s1/tasks/t/stop", true)
            .await
            .0,
        502
    );
    assert_eq!(
        f.request("POST", "/api/sessions/s1/subagents/a/cancel", true)
            .await
            .0,
        502
    );
    assert!(f.calls("_x.ai/subagent/cancel").is_empty());
    f.set(json!({"_x.ai/subagent/get":{"snapshot":agent("s1","running")},"_x.ai/subagent/cancel":{"subagentId":"a","cancelled":true,"outcome":{"kind":"not_found"}}}));
    assert_eq!(
        f.request("POST", "/api/sessions/s1/subagents/a/cancel", true)
            .await
            .0,
        502
    );
}

#[tokio::test]
async fn task_visibility_and_scoped_kill_capability() {
    let f = Fixture::new().await;
    let mut sibling = task(false);
    sibling["task_id"] = json!("sibling");
    sibling["owner_session_id"] = json!("s2");
    let mut foreground = task(false);
    foreground["task_id"] = json!("foreground");
    foreground["is_backgrounded"] = json!(false);
    let mut missing = task(false);
    missing["task_id"] = json!("missing");
    missing.as_object_mut().unwrap().remove("owner_session_id");
    let mut null_owner = task(false);
    null_owner["task_id"] = json!("null");
    null_owner["owner_session_id"] = Value::Null;
    let mut camel = task(false);
    camel["task_id"] = json!("camel");
    camel.as_object_mut().unwrap().remove("owner_session_id");
    camel.as_object_mut().unwrap().remove("is_backgrounded");
    camel["ownerSessionId"] = json!("s1");
    camel["isBackgrounded"] = json!(true);
    f.set(json!({"_x.ai/task/list":{"tasks":[task(false),sibling,foreground,missing,null_owner,camel]}}));
    let (status, body) = f.request("GET", "/api/sessions/s1/tasks", true).await;
    assert_eq!(status, 200);
    let tasks = body["tasks"].as_array().unwrap();
    assert_eq!(tasks.len(), 2);
    assert_eq!(tasks[0]["description"], "real label");
    assert_eq!(tasks[0]["canStop"], false);
    assert_eq!(tasks[1]["taskId"], "camel");
    for id in ["sibling", "foreground", "missing", "null"] {
        assert_eq!(
            f.request("POST", &format!("/api/sessions/s1/tasks/{id}/stop"), true)
                .await
                .0,
            404
        );
    }
    let (status, body) = f
        .request("POST", "/api/sessions/s1/tasks/t/stop", true)
        .await;
    assert_eq!(status, 409);
    assert!(body["error"].as_str().unwrap().contains("unsupported"));
    assert!(f.calls("_x.ai/task/kill").is_empty());
    assert!(f.calls("_x.ai/task/kill_scoped").is_empty());
    f.set(json!({"_x.ai/task/list":{"scopedKillSupported":true,"tasks":[task(false)]},"_x.ai/task/kill_scoped":{"taskId":"t","outcome":"killed"}}));
    assert_eq!(
        f.request("GET", "/api/sessions/s1/tasks", true).await.1["tasks"][0]["canStop"],
        true
    );
    assert_eq!(
        f.request("POST", "/api/sessions/s1/tasks/t/stop", true)
            .await
            .1["outcome"],
        "killed"
    );
    let calls = f.calls("_x.ai/task/kill_scoped");
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0]["params"],
        json!({"sessionId":"s1","taskId":"t","source":"clientUi"})
    );
    assert!(f.calls("_x.ai/task/kill").is_empty());
}
