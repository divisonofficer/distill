//! Keeper: one long-lived agent connection that tracks the session list, keeps remembered
//! sessions attached, and turns `pending_interaction` / `interaction_resolved` into board events.
//! Public API: plan/research-base/CONTRACT.md §1, §4, §8.3.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use anyhow::anyhow;
use serde_json::{Value, json};
use tokio::sync::{broadcast, mpsc};

use crate::agent::{self, AgentConfig, AgentHandle, lock};

const MAX_BACKOFF: Duration = Duration::from_secs(30);
const EVENT_CAPACITY: usize = 256;
static NULL: Value = Value::Null;

#[derive(Debug, Clone)]
pub enum BoardEvent {
    Upsert(Vec<Value>),
    Remove(Vec<String>),
}

#[derive(Debug, Clone)]
pub struct NeedsInput {
    pub session_id: String,
    pub tool_call_id: String,
    pub kind: String,
}

#[derive(Clone)]
pub struct Keeper {
    inner: Arc<Inner>,
}

struct Inner {
    cfg: AgentConfig,
    conn: Mutex<AgentHandle>,
    state: Mutex<State>,
    attach_gate: tokio::sync::Mutex<()>,
    board: broadcast::Sender<BoardEvent>,
    needs: broadcast::Sender<NeedsInput>,
}

#[derive(Default)]
struct State {
    rows: Vec<Value>,
    /// Sessions loaded on this keeper, in attach order. Re-attached after a respawn.
    attached: Vec<String>,
    /// Tool calls already announced as `NeedsInput`.
    asked: HashSet<String>,
}

impl Keeper {
    pub async fn start(cfg: AgentConfig) -> anyhow::Result<Self> {
        let (handle, rx) = agent::spawn(&cfg).await?;
        handle.initialize().await?;
        let (board, _) = broadcast::channel(EVENT_CAPACITY);
        let (needs, _) = broadcast::channel(EVENT_CAPACITY);
        let inner = Arc::new(Inner {
            cfg,
            conn: Mutex::new(handle),
            state: Mutex::new(State::default()),
            attach_gate: tokio::sync::Mutex::new(()),
            board,
            needs,
        });
        let keeper = Self { inner };
        keeper.refresh().await?;
        tokio::spawn(run(Arc::downgrade(&keeper.inner), rx));
        Ok(keeper)
    }

    pub fn handle(&self) -> AgentHandle {
        lock(&self.inner.conn).clone()
    }

    pub async fn refresh(&self) -> anyhow::Result<Vec<Value>> {
        let mut rows = list(&self.handle()).await?;
        let mut state = lock(&self.inner.state);
        let pending: Vec<Value> = state
            .rows
            .iter()
            .filter(|r| {
                let sid = r.get("sessionId").and_then(Value::as_str);
                state.attached.iter().any(|a| Some(a.as_str()) == sid)
                    && !rows
                        .iter()
                        .any(|n| n.get("sessionId").and_then(Value::as_str) == sid)
            })
            .cloned()
            .collect();
        rows.extend(pending);
        state.rows = rows.clone();
        Ok(rows)
    }

    pub fn rows(&self) -> Vec<Value> {
        lock(&self.inner.state).rows.clone()
    }

    pub fn cwd_of(&self, session_id: &str) -> Option<PathBuf> {
        let state = lock(&self.inner.state);
        let row = state
            .rows
            .iter()
            .find(|r| r.get("sessionId").and_then(Value::as_str) == Some(session_id))?;
        row.get("cwd").and_then(Value::as_str).map(PathBuf::from)
    }

    pub async fn attach(&self, session_id: &str) -> anyhow::Result<()> {
        let _gate = self.inner.attach_gate.lock().await;
        if lock(&self.inner.state)
            .attached
            .iter()
            .any(|s| s == session_id)
        {
            return Ok(());
        }
        let cwd = match self.cwd_of(session_id) {
            Some(cwd) => cwd,
            None => {
                self.refresh().await?;
                self.cwd_of(session_id)
                    .ok_or_else(|| anyhow!("unknown session {session_id}"))?
            }
        };
        load(&self.handle(), session_id, &cwd).await?;
        lock(&self.inner.state).attached.push(session_id.to_owned());
        Ok(())
    }

    /// Record a session this keeper created with `session/new`: it is already loaded on this
    /// connection, and the leader lists it only after its first turn, so seed a row with its cwd.
    pub fn adopt(&self, session_id: &str, cwd: &Path) {
        let mut state = lock(&self.inner.state);
        if !state.attached.iter().any(|s| s == session_id) {
            state.attached.push(session_id.to_owned());
        }
        if !state
            .rows
            .iter()
            .any(|r| r.get("sessionId").and_then(Value::as_str) == Some(session_id))
        {
            state.rows.push(json!({
                "sessionId": session_id,
                "cwd": cwd.to_string_lossy(),
                "activity": "idle",
                "resident": true,
            }));
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<BoardEvent> {
        self.inner.board.subscribe()
    }

    pub fn needs_input(&self) -> broadcast::Receiver<NeedsInput> {
        self.inner.needs.subscribe()
    }
}

impl Inner {
    /// Spawn a new child and restore state on it. Commits the new connection only on success.
    async fn connect(&self) -> anyhow::Result<mpsc::UnboundedReceiver<Value>> {
        let (handle, rx) = agent::spawn(&self.cfg).await?;
        handle.initialize().await?;
        let rows = list(&handle).await?;
        lock(&self.state).rows = rows.clone();
        let attached = lock(&self.state).attached.clone();
        for sid in attached {
            let cwd = rows.iter().find_map(|r| {
                (r.get("sessionId").and_then(Value::as_str) == Some(sid.as_str()))
                    .then(|| r.get("cwd").and_then(Value::as_str).map(PathBuf::from))
                    .flatten()
            });
            match cwd {
                Some(cwd) => {
                    if let Err(e) = load(&handle, &sid, &cwd).await {
                        tracing::warn!(error = %e, session = %sid, "re-attach failed");
                    }
                }
                None => tracing::warn!(session = %sid, "re-attach skipped: no cwd"),
            }
        }
        *lock(&self.conn) = handle;
        Ok(rx)
    }

    fn apply(&self, msg: &Value) {
        let Some((method, params)) = envelope(msg) else {
            return;
        };
        match method {
            "x.ai/sessions/changed" => self.apply_changed(params),
            "x.ai/session_notification" => self.apply_update(params),
            _ => {}
        }
    }

    fn apply_changed(&self, params: &Value) {
        let upserted: Vec<Value> = params
            .get("upserted")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let removed: Vec<String> = params
            .get("removed")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        {
            let mut state = lock(&self.state);
            for row in &upserted {
                let Some(sid) = row.get("sessionId").and_then(Value::as_str) else {
                    continue;
                };
                match state
                    .rows
                    .iter_mut()
                    .find(|r| r.get("sessionId").and_then(Value::as_str) == Some(sid))
                {
                    Some(slot) => *slot = row.clone(),
                    None => state.rows.push(row.clone()),
                }
            }
            state.rows.retain(|r| {
                !removed
                    .iter()
                    .any(|id| r.get("sessionId").and_then(Value::as_str) == Some(id.as_str()))
            });
        }
        if !upserted.is_empty() {
            let _ = self.board.send(BoardEvent::Upsert(upserted));
        }
        if !removed.is_empty() {
            let _ = self.board.send(BoardEvent::Remove(removed));
        }
    }

    fn apply_update(&self, params: &Value) {
        let Some(sid) = params.get("sessionId").and_then(Value::as_str) else {
            return;
        };
        let update = params.get("update").unwrap_or(&NULL);
        match update.get("sessionUpdate").and_then(Value::as_str) {
            Some("pending_interaction") => {
                if let Some(row) = self.set_activity(sid, "needs_input") {
                    let _ = self.board.send(BoardEvent::Upsert(vec![row]));
                }
                let tool_call_id = update
                    .get("tool_call_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if lock(&self.state).asked.insert(tool_call_id.to_owned()) {
                    let _ = self.needs.send(NeedsInput {
                        session_id: sid.to_owned(),
                        tool_call_id: tool_call_id.to_owned(),
                        kind: update
                            .get("kind")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                    });
                }
            }
            Some("interaction_resolved") => {
                if let Some(row) = self.set_activity(sid, "working") {
                    let _ = self.board.send(BoardEvent::Upsert(vec![row]));
                }
            }
            _ => {}
        }
    }

    fn set_activity(&self, sid: &str, activity: &str) -> Option<Value> {
        let mut state = lock(&self.state);
        let row = state
            .rows
            .iter_mut()
            .find(|r| r.get("sessionId").and_then(Value::as_str) == Some(sid))?;
        row["activity"] = json!(activity);
        Some(row.clone())
    }
}

/// Consume agent messages; when the child dies, reconnect with backoff.
async fn run(weak: Weak<Inner>, mut rx: mpsc::UnboundedReceiver<Value>) {
    loop {
        while let Some(msg) = rx.recv().await {
            let Some(inner) = weak.upgrade() else {
                return;
            };
            inner.apply(&msg);
        }
        match reconnect(&weak).await {
            Some(next) => rx = next,
            None => return,
        }
    }
}

async fn reconnect(weak: &Weak<Inner>) -> Option<mpsc::UnboundedReceiver<Value>> {
    let mut backoff = Duration::from_secs(1);
    loop {
        tokio::time::sleep(backoff).await;
        let inner = weak.upgrade()?;
        match inner.connect().await {
            Ok(rx) => return Some(rx),
            Err(e) => tracing::warn!(error = %e, ?backoff, "agent reconnect failed"),
        }
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}

async fn list(handle: &AgentHandle) -> anyhow::Result<Vec<Value>> {
    let res = handle.request("_x.ai/sessions/list", json!({})).await?;
    Ok(res
        .get("sessions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default())
}

async fn load(handle: &AgentHandle, session_id: &str, cwd: &Path) -> anyhow::Result<()> {
    handle
        .request(
            "session/load",
            json!({"sessionId": session_id, "cwd": cwd.to_string_lossy(), "mcpServers": []}),
        )
        .await?;
    Ok(())
}

/// Method name without the leading `_`, and params of the inner call when wrapped (CONTRACT §8.3).
fn envelope(msg: &Value) -> Option<(&str, &Value)> {
    let method = msg.get("method")?.as_str()?;
    let params = msg.get("params").unwrap_or(&NULL);
    if let (Some(inner), Some(inner_params)) = (
        params.get("method").and_then(Value::as_str),
        params.get("params"),
    ) {
        return Some((inner.strip_prefix('_').unwrap_or(inner), inner_params));
    }
    Some((method.strip_prefix('_').unwrap_or(method), params))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_unwraps_and_strips_prefix() {
        let plain = json!({"method": "_x.ai/sessions/changed", "params": {"removed": ["a"]}});
        let (m, p) = envelope(&plain).unwrap();
        assert_eq!(m, "x.ai/sessions/changed");
        assert_eq!(p["removed"][0], "a");

        let wrapped = json!({"method": "_x.ai/session_notification",
            "params": {"method": "x.ai/session_notification", "params": {"sessionId": "s1"}}});
        let (m, p) = envelope(&wrapped).unwrap();
        assert_eq!(m, "x.ai/session_notification");
        assert_eq!(p["sessionId"], "s1");
    }
}
