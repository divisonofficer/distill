//! Connection to the local Distill leader: one `distill agent --leader stdio` child, driven as an actor.
//! Public API: plan/research-base/CONTRACT.md §4, §8.1.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use anyhow::{Context as _, anyhow, bail};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::process::{Child, ChildStdout, Command};
use tokio::sync::{mpsc, oneshot};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub distill_bin: PathBuf,
    pub args: Vec<OsString>,
    pub extra_env: Vec<(OsString, OsString)>,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            distill_bin: PathBuf::from("distill"),
            args: ["agent", "--leader", "stdio"].map(OsString::from).to_vec(),
            extra_env: Vec::new(),
        }
    }
}

type Pending = Mutex<Option<HashMap<u64, oneshot::Sender<Value>>>>;

#[derive(Clone)]
pub struct AgentHandle {
    inner: Arc<Shared>,
}

struct Shared {
    lines: mpsc::UnboundedSender<String>,
    /// `None` once the child's stdout closed: new requests fail at once.
    pending: Arc<Pending>,
    next_id: AtomicU64,
    /// Dropping the last handle drops this sender and stops the reader task, which kills the child.
    _kill: oneshot::Sender<()>,
}

/// Starts the child and returns the handle plus the queue of agent→client messages (no responses).
pub async fn spawn(
    cfg: &AgentConfig,
) -> anyhow::Result<(AgentHandle, mpsc::UnboundedReceiver<Value>)> {
    let mut cmd = Command::new(&cfg.distill_bin);
    cmd.args(&cfg.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true);
    for (key, value) in &cfg.extra_env {
        cmd.env(key, value);
    }
    #[allow(
        clippy::disallowed_methods,
        reason = "child is held by the reader task and killed on drop"
    )]
    let mut child = cmd
        .spawn()
        .with_context(|| format!("spawn {}", cfg.distill_bin.display()))?;
    let mut stdin = child.stdin.take().context("agent stdin not piped")?;
    let stdout = child.stdout.take().context("agent stdout not piped")?;

    let (line_tx, mut line_rx) = mpsc::unbounded_channel::<String>();
    tokio::spawn(async move {
        while let Some(line) = line_rx.recv().await {
            if stdin.write_all(line.as_bytes()).await.is_err() || stdin.flush().await.is_err() {
                break;
            }
        }
    });

    let pending: Arc<Pending> = Arc::new(Mutex::new(Some(HashMap::new())));
    let (inbound_tx, inbound_rx) = mpsc::unbounded_channel();
    let (kill_tx, kill_rx) = oneshot::channel::<()>();
    tokio::spawn(read_loop(
        child,
        stdout,
        pending.clone(),
        inbound_tx,
        kill_rx,
    ));

    let handle = AgentHandle {
        inner: Arc::new(Shared {
            lines: line_tx,
            pending,
            next_id: AtomicU64::new(1),
            _kill: kill_tx,
        }),
    };
    Ok((handle, inbound_rx))
}

async fn read_loop(
    _child: Child,
    stdout: ChildStdout,
    pending: Arc<Pending>,
    inbound: mpsc::UnboundedSender<Value>,
    mut kill: oneshot::Receiver<()>,
) {
    let mut lines = BufReader::new(stdout).lines();
    loop {
        let line = tokio::select! {
            _ = &mut kill => break,
            line = lines.next_line() => line,
        };
        let line = match line {
            Ok(Some(line)) => line,
            Ok(None) => break,
            Err(e) => {
                tracing::warn!(error = %e, "agent stdout read failed");
                break;
            }
        };
        let msg: Value = match serde_json::from_str(&line) {
            Ok(msg) => msg,
            Err(e) => {
                tracing::warn!(error = %e, "agent: skipped non-JSON line");
                continue;
            }
        };
        if is_response(&msg) {
            resolve(&pending, msg);
        } else {
            let _ = inbound.send(msg);
        }
    }
    // Dropping the senders wakes every waiter with a closed-connection error.
    lock(&pending).take();
}

fn is_response(msg: &Value) -> bool {
    msg.get("id").is_some()
        && msg.get("method").is_none()
        && (msg.get("result").is_some() || msg.get("error").is_some())
}

fn resolve(pending: &Pending, msg: Value) {
    let Some(id) = msg.get("id").and_then(Value::as_u64) else {
        return;
    };
    if let Some(tx) = lock(pending).as_mut().and_then(|map| map.remove(&id)) {
        let _ = tx.send(msg);
    }
}

impl AgentHandle {
    pub async fn request(&self, method: &str, params: Value) -> anyhow::Result<Value> {
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        match lock(&self.inner.pending).as_mut() {
            Some(map) => {
                map.insert(id, tx);
            }
            None => bail!("agent connection closed"),
        }
        if let Err(e) =
            self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
        {
            self.forget(id);
            return Err(e);
        }
        let msg = match tokio::time::timeout(REQUEST_TIMEOUT, rx).await {
            Ok(Ok(msg)) => msg,
            Ok(Err(_)) => bail!("{method}: agent connection closed"),
            Err(_) => {
                self.forget(id);
                bail!("{method}: no response in {}s", REQUEST_TIMEOUT.as_secs())
            }
        };
        if let Some(error) = msg.get("error") {
            bail!("{method} failed: {error}");
        }
        Ok(msg.get("result").cloned().unwrap_or(Value::Null))
    }

    pub fn notify(&self, method: &str, params: Value) -> anyhow::Result<()> {
        self.send(json!({"jsonrpc": "2.0", "method": method, "params": params}))
    }

    pub fn respond(&self, id: Value, result: Value) -> anyhow::Result<()> {
        self.send(json!({"jsonrpc": "2.0", "id": id, "result": result}))
    }

    /// `initialize` then `authenticate`, same params as `RawClient::initialize` in postudio-probe.
    pub async fn initialize(&self) -> anyhow::Result<Value> {
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
            .ok_or_else(|| anyhow!("no usable auth method in {offered:?}"))?;
        self.request(
            "authenticate",
            json!({"methodId": method, "_meta": {"headless": true}}),
        )
        .await?;
        Ok(init)
    }

    fn send(&self, msg: Value) -> anyhow::Result<()> {
        self.inner
            .lines
            .send(format!("{msg}\n"))
            .map_err(|_| anyhow!("agent connection closed"))
    }

    fn forget(&self, id: u64) {
        if let Some(map) = lock(&self.inner.pending).as_mut() {
            map.remove(&id);
        }
    }
}

pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}
