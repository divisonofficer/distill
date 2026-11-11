//! Raw line-delimited JSON-RPC client for `distill agent --leader stdio`, plus a leader harness on the mock model server.
//! Every line in either direction lands in an ordered log as `{"dir":"c2a"|"a2c","msg":..}`.

pub mod stack;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use distill_test_support::process::{TestOutput, TestProcess, TestProcessConfig, TestStdin};
use distill_test_support::{LeaderFixture, MockInferenceServer, TestSandbox, grok_binary};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::sync::mpsc;

pub const DEFAULT_WAIT: Duration = Duration::from_secs(30);

type Log = Arc<Mutex<Vec<Value>>>;

fn push_log(log: &Log, dir: &str, msg: &Value) {
    log.lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(json!({ "dir": dir, "msg": msg }));
}

/// A leader on a private sandbox and mock server. Field order is drop order: leader first.
pub struct Harness {
    pub leader: LeaderFixture,
    pub server: MockInferenceServer,
    pub sandbox: TestSandbox,
    pub cwd: PathBuf,
}

impl Harness {
    pub async fn start() -> Self {
        let server = MockInferenceServer::start().await.expect("mock server");
        let sandbox = TestSandbox::builder().mock_url(server.url()).build();
        let cwd = sandbox.root().join("proj");
        std::fs::create_dir_all(&cwd).expect("create cwd");
        let leader = LeaderFixture::start(&server, sandbox.workspace(), &sandbox)
            .await
            .expect("start leader");
        Self {
            leader,
            server,
            sandbox,
            cwd,
        }
    }

    pub fn spawn_client(&self) -> RawClient {
        let base_url = self.server.url();
        let mut cmd = tokio::process::Command::new(grok_binary());
        cmd.args(["agent", "--leader", "stdio"])
            .current_dir(&self.cwd);
        let socket = self.sandbox.distill_home().join("leader.sock");
        let mut process = TestProcess::spawn(
            cmd,
            &self.sandbox,
            TestProcessConfig::new()
                .label("probe stdio client")
                .stdin(TestStdin::Piped)
                .stdout(TestOutput::Piped)
                .env("GROK_CLI_CHAT_PROXY_BASE_URL", &base_url)
                .env("GROK_XAI_API_BASE_URL", &base_url)
                .env("GROK_MODELS_BASE_URL", &base_url)
                .env("GROK_FEEDBACK_BASE_URL", &base_url)
                .env("GROK_TRACE_UPLOAD_URL", &base_url)
                .env("XAI_API_KEY", "test-key-for-ci")
                .env("GROK_LEADER_SOCKET", socket)
                .env("RUST_LOG", "distill_shell=debug,distill_login=debug"),
        )
        .expect("spawn client");
        let stdin = process.take_stdin().expect("stdin");
        let stdout = process.take_stdout().expect("stdout");
        let log: Log = Arc::default();
        let (tx, rx) = mpsc::unbounded_channel();
        let reader_log = log.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                match serde_json::from_str::<Value>(&line) {
                    Ok(msg) => {
                        push_log(&reader_log, "a2c", &msg);
                        if tx.send(msg).is_err() {
                            break;
                        }
                    }
                    Err(_) => eprintln!("non-json stdout line: {line}"),
                }
            }
        });
        RawClient {
            process,
            stdin: Some(stdin),
            rx,
            seen: Vec::new(),
            taken: Vec::new(),
            log,
            next_id: 1,
        }
    }

    /// Replace machine-specific strings so a fixture is stable across hosts.
    pub fn sanitize(&self, text: &str) -> String {
        let root = self.sandbox.root().to_string_lossy().into_owned();
        let mut roots = Vec::new();
        if root.starts_with("/var/") {
            roots.push(format!("/private{root}"));
        }
        roots.push(root);
        let mut out = text.to_owned();
        for r in roots {
            out = out.replace(&r, "/sandbox");
        }
        if let Ok(home) = std::env::var("HOME")
            && home.len() > 1
        {
            out = out.replace(&home, "/home/user");
        }
        out.replace(&self.server.origin(), "http://mock")
    }
}

pub struct RawClient {
    process: TestProcess,
    stdin: Option<tokio::process::ChildStdin>,
    rx: mpsc::UnboundedReceiver<Value>,
    seen: Vec<Value>,
    taken: Vec<bool>,
    log: Log,
    next_id: u64,
}

impl RawClient {
    pub fn log(&self) -> Vec<Value> {
        self.log.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn stderr(&self) -> String {
        self.process.stderr_tail().text
    }

    pub async fn send(&mut self, msg: Value) {
        push_log(&self.log, "c2a", &msg);
        let stdin = self.stdin.as_mut().expect("client stdin closed");
        let mut line = msg.to_string();
        line.push('\n');
        stdin.write_all(line.as_bytes()).await.expect("write stdin");
        stdin.flush().await.expect("flush stdin");
    }

    pub async fn notify(&mut self, method: &str, params: Value) {
        self.send(json!({"jsonrpc":"2.0","method":method,"params":params}))
            .await;
    }

    /// Send a request without waiting; returns its id.
    pub async fn send_request(&mut self, method: &str, params: Value) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))
            .await;
        id
    }

    pub async fn await_response(&mut self, id: u64, timeout: Duration) -> Result<Value, String> {
        self.wait_for(&format!("response id={id}"), timeout, |m| {
            m.get("method").is_none() && m.get("id") == Some(&json!(id))
        })
        .await
    }

    /// Request + response; an error response is returned as `Err`.
    pub async fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.send_request(method, params).await;
        let resp = self.await_response(id, DEFAULT_WAIT).await?;
        match resp.get("error") {
            Some(err) => Err(format!("{method} -> error {err}")),
            None => Ok(resp.get("result").cloned().unwrap_or(Value::Null)),
        }
    }

    /// Next incoming message matching `pred` that no earlier wait returned.
    pub async fn wait_for(
        &mut self,
        what: &str,
        timeout: Duration,
        pred: impl Fn(&Value) -> bool,
    ) -> Result<Value, String> {
        let deadline = tokio::time::Instant::now() + timeout;
        let mut scanned = 0;
        loop {
            while scanned < self.seen.len() {
                if !self.taken[scanned] && pred(&self.seen[scanned]) {
                    self.taken[scanned] = true;
                    return Ok(self.seen[scanned].clone());
                }
                scanned += 1;
            }
            match tokio::time::timeout_at(deadline, self.rx.recv()).await {
                Ok(Some(msg)) => {
                    self.seen.push(msg);
                    self.taken.push(false);
                }
                Ok(None) => return Err(format!("{what}: agent closed stdout\n{}", self.summary())),
                Err(_) => {
                    return Err(format!(
                        "timed out after {timeout:?} waiting for {what}\n{}",
                        self.summary()
                    ));
                }
            }
        }
    }

    /// Read incoming messages for `dur` without consuming any.
    pub async fn pump_for(&mut self, dur: Duration) {
        let deadline = tokio::time::Instant::now() + dur;
        while let Ok(Some(msg)) = tokio::time::timeout_at(deadline, self.rx.recv()).await {
            self.seen.push(msg);
            self.taken.push(false);
        }
    }

    /// Every incoming message so far (consumed or not).
    pub fn incoming(&self) -> &[Value] {
        &self.seen
    }

    pub fn summary(&self) -> String {
        let methods: Vec<String> = self
            .seen
            .iter()
            .map(|m| method_of(m).unwrap_or("<response>").to_owned())
            .collect();
        format!(
            "incoming methods so far: {methods:?}\nstderr tail: {}",
            self.stderr()
        )
    }

    pub async fn initialize(&mut self) -> Result<Value, String> {
        let init = self
            .request(
                "initialize",
                json!({
                    "protocolVersion": 1,
                    "clientCapabilities": {"fs": {"readTextFile": false, "writeTextFile": false}, "terminal": false},
                    "_meta": {
                        "startupHints": {"nonInteractive": false, "skipGitStatus": true, "skipProjectLayout": true},
                        "clientType": "test-client",
                        "clientVersion": "0.0.0-test"
                    }
                }),
            )
            .await?;
        let offered: Vec<&str> = init
            .get("authMethods")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|m| m.get("id")?.as_str()).collect())
            .unwrap_or_default();
        let method = ["xai.api_key", "cached_token"]
            .into_iter()
            .find(|w| offered.contains(w))
            .ok_or_else(|| format!("no usable auth method in {offered:?}"))?;
        self.request(
            "authenticate",
            json!({"methodId": method, "_meta": {"headless": true}}),
        )
        .await?;
        Ok(init)
    }

    pub async fn new_session(&mut self, cwd: &Path) -> Result<String, String> {
        let result = self
            .request("session/new", json!({"cwd": cwd, "mcpServers": []}))
            .await?;
        result
            .get("sessionId")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| format!("session/new result has no sessionId: {result}"))
    }

    pub async fn load_session(&mut self, sid: &str, cwd: &Path) -> Result<Value, String> {
        self.request(
            "session/load",
            json!({"sessionId": sid, "cwd": cwd, "mcpServers": []}),
        )
        .await
    }

    pub async fn send_prompt(&mut self, sid: &str, text: &str) -> u64 {
        self.send_request(
            "session/prompt",
            json!({"sessionId": sid, "prompt": [{"type": "text", "text": text}]}),
        )
        .await
    }

    pub async fn wait_permission(&mut self, sid: &str, timeout: Duration) -> Result<Value, String> {
        let sid = sid.to_owned();
        self.wait_for("session/request_permission", timeout, move |m| {
            method_of(m) == Some("session/request_permission")
                && m.pointer("/params/sessionId").and_then(Value::as_str) == Some(sid.as_str())
        })
        .await
    }

    /// Answer a `session/request_permission` request with the first option of `kind` (e.g. "allow_once").
    pub async fn answer_permission(&mut self, request: &Value, kind: &str) -> Result<(), String> {
        let option_id = request
            .pointer("/params/options")
            .and_then(Value::as_array)
            .and_then(|opts| {
                opts.iter()
                    .find(|o| o.get("kind").and_then(Value::as_str) == Some(kind))
            })
            .and_then(|o| o.get("optionId")?.as_str())
            .ok_or_else(|| format!("no {kind} option in {request}"))?
            .to_owned();
        self.send(json!({
            "jsonrpc": "2.0",
            "id": request.get("id").cloned().unwrap_or(Value::Null),
            "result": {"outcome": {"outcome": "selected", "optionId": option_id}}
        }))
        .await;
        Ok(())
    }

    /// Mark everything received so far as consumed, after letting in-flight replay settle.
    pub async fn settle(&mut self) {
        self.pump_for(Duration::from_millis(700)).await;
        self.taken.iter_mut().for_each(|t| *t = true);
    }

    /// Answer every permission request with `kind` until the response to `prompt_id` arrives.
    /// Returns the response and the number of permission requests answered.
    pub async fn finish_turn(
        &mut self,
        prompt_id: u64,
        kind: &str,
    ) -> Result<(Value, usize), String> {
        let mut answered = 0;
        loop {
            let msg = self
                .wait_for(
                    "prompt response or permission request",
                    Duration::from_secs(60),
                    |m| {
                        (m.get("method").is_none() && m.get("id") == Some(&json!(prompt_id)))
                            || method_of(m) == Some("session/request_permission")
                    },
                )
                .await?;
            if method_of(&msg).is_none() {
                return Ok((msg, answered));
            }
            self.answer_permission(&msg, kind).await?;
            answered += 1;
        }
    }

    /// Drop stdin and stop the child.
    pub async fn close(&mut self) {
        self.stdin = None;
        let _ = self.process.kill().await;
    }
}

pub fn method_of(msg: &Value) -> Option<&str> {
    msg.get("method")?.as_str()
}

/// `params.update.sessionUpdate` of a `session/update` or `_x.ai/session_notification` notification.
pub fn update_kind(msg: &Value) -> Option<&str> {
    if !matches!(
        method_of(msg),
        Some("session/update" | "_x.ai/session_notification")
    ) {
        return None;
    }
    msg.pointer("/params/update/sessionUpdate")?.as_str()
}

/// `activity` values of roster rows (any nesting) that belong to `sid`.
pub fn activities_for(msg: &Value, sid: &str) -> Vec<String> {
    fn walk(v: &Value, sid: &str, out: &mut Vec<String>) {
        match v {
            Value::Object(map) => {
                if map.get("sessionId").and_then(Value::as_str) == Some(sid)
                    && let Some(a) = map.get("activity").and_then(Value::as_str)
                {
                    out.push(a.to_owned());
                }
                map.values().for_each(|c| walk(c, sid, out));
            }
            Value::Array(items) => items.iter().for_each(|c| walk(c, sid, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(msg, sid, &mut out);
    out
}

pub fn pid_alive(pid: u32) -> bool {
    // SAFETY: signal 0 only checks existence.
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

pub fn kill_pid(pid: u32) {
    // SAFETY: best-effort cleanup of a pid this test started.
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGKILL);
    }
}

/// Kills the pid on drop so a failed probe leaves no background process.
pub struct PidGuard(pub u32);

impl Drop for PidGuard {
    fn drop(&mut self) {
        kill_pid(self.0);
    }
}

/// Unwrap a probe step, failing the test with the observed message.
pub fn must<T>(what: &str, result: Result<T, String>) -> T {
    result.unwrap_or_else(|e| panic!("{what}: {e}"))
}
