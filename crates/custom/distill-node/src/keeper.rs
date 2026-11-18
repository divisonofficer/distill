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
use crate::sep::PermissionMode;

const MAX_BACKOFF: Duration = Duration::from_secs(30);
const REFRESH_INTERVAL: Duration = Duration::from_secs(2);
const EVENT_CAPACITY: usize = 256;
pub const MAX_CHAT_PROGRESS_BYTES: usize = 1024 * 1024;
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

#[derive(Debug, Clone)]
pub struct ChatProgress {
    pub request_id: String,
    pub text: String,
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
    chat_progress: broadcast::Sender<ChatProgress>,
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
        let (chat_progress, _) = broadcast::channel(EVENT_CAPACITY);
        let inner = Arc::new(Inner {
            cfg,
            conn: Mutex::new(handle),
            state: Mutex::new(State::default()),
            attach_gate: tokio::sync::Mutex::new(()),
            board,
            needs,
            chat_progress,
        });
        let keeper = Self { inner };
        keeper.refresh().await?;
        for row in keeper.rows() {
            let keep = row.get("resident").and_then(Value::as_bool) == Some(true)
                || matches!(
                    row.get("activity").and_then(Value::as_str),
                    Some("working" | "needs_input")
                );
            if keep
                && let Some(session_id) = row.get("sessionId").and_then(Value::as_str)
                && let Err(e) = keeper.attach(session_id).await
            {
                tracing::warn!(error = %e, session = %session_id, "startup attach failed");
            }
        }
        tokio::spawn(run(Arc::downgrade(&keeper.inner), rx));
        Ok(keeper)
    }

    pub fn handle(&self) -> AgentHandle {
        lock(&self.inner.conn).clone()
    }

    pub async fn refresh(&self) -> anyhow::Result<Vec<Value>> {
        self.inner.refresh().await?;
        Ok(self.rows())
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
    pub fn adopt(&self, session_id: &str, cwd: &Path, mode: PermissionMode) {
        let mut state = lock(&self.inner.state);
        if !state.attached.iter().any(|s| s == session_id) {
            state.attached.push(session_id.to_owned());
        }
        let (yolo, auto_mode) = mode_flags(mode);
        let row = match state
            .rows
            .iter_mut()
            .find(|r| r.get("sessionId").and_then(Value::as_str) == Some(session_id))
        {
            Some(row) => {
                row["yolo"] = json!(yolo);
                row["autoMode"] = json!(auto_mode);
                row.clone()
            }
            None => {
                let row = json!({
                    "sessionId": session_id,
                    "cwd": cwd.to_string_lossy(),
                    "activity": "idle",
                    "resident": true,
                    "yolo": yolo,
                    "autoMode": auto_mode,
                });
                state.rows.push(row.clone());
                row
            }
        };
        drop(state);
        let _ = self.inner.board.send(BoardEvent::Upsert(vec![row]));
    }

    pub fn subscribe(&self) -> broadcast::Receiver<BoardEvent> {
        self.inner.board.subscribe()
    }

    pub fn needs_input(&self) -> broadcast::Receiver<NeedsInput> {
        self.inner.needs.subscribe()
    }

    pub fn chat_progress(&self) -> broadcast::Receiver<ChatProgress> {
        self.inner.chat_progress.subscribe()
    }
}

impl Inner {
    async fn refresh(&self) -> anyhow::Result<()> {
        let handle = lock(&self.conn).clone();
        let rows = list(&handle).await?;
        self.replace_rows(rows);
        Ok(())
    }

    fn replace_rows(&self, rows: Vec<Value>) {
        let (upserted, removed) = {
            let mut state = lock(&self.state);
            replace_rows(&mut state, rows)
        };
        if !upserted.is_empty() {
            let _ = self.board.send(BoardEvent::Upsert(upserted));
        }
        if !removed.is_empty() {
            let _ = self.board.send(BoardEvent::Remove(removed));
        }
    }

    /// Spawn a new child and restore state on it. Commits the new connection only on success.
    async fn connect(&self) -> anyhow::Result<mpsc::UnboundedReceiver<Value>> {
        let (handle, rx) = agent::spawn(&self.cfg).await?;
        handle.initialize().await?;
        let rows = list(&handle).await?;
        self.replace_rows(rows);
        let rows = lock(&self.state).rows.clone();
        let attached = lock(&self.state).attached.clone();
        let mut restored = Vec::new();
        for sid in attached {
            let cwd = rows.iter().find_map(|r| {
                (r.get("sessionId").and_then(Value::as_str) == Some(sid.as_str()))
                    .then(|| r.get("cwd").and_then(Value::as_str).map(PathBuf::from))
                    .flatten()
            });
            match cwd {
                Some(cwd) => match load(&handle, &sid, &cwd).await {
                    Ok(()) => restored.push(sid),
                    Err(e) => tracing::warn!(error = %e, session = %sid, "re-attach failed"),
                },
                None => tracing::warn!(session = %sid, "re-attach skipped: no cwd"),
            }
        }
        lock(&self.state).attached = restored;
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
            "x.ai/chat/progress" => self.apply_chat_progress(params),
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

    fn apply_chat_progress(&self, params: &Value) {
        let (Some(request_id), Some(text)) = (
            params.get("requestId").and_then(Value::as_str),
            params.get("text").and_then(Value::as_str),
        ) else {
            return;
        };
        if request_id.is_empty() || text.len() > MAX_CHAT_PROGRESS_BYTES {
            return;
        }
        let _ = self.chat_progress.send(ChatProgress {
            request_id: request_id.to_owned(),
            text: text.to_owned(),
        });
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

fn replace_rows(state: &mut State, mut rows: Vec<Value>) -> (Vec<Value>, Vec<String>) {
    let pending: Vec<Value> = state
        .rows
        .iter()
        .filter(|row| {
            let sid = row.get("sessionId").and_then(Value::as_str);
            state
                .attached
                .iter()
                .any(|attached| Some(attached.as_str()) == sid)
                && !rows
                    .iter()
                    .any(|new| new.get("sessionId").and_then(Value::as_str) == sid)
        })
        .cloned()
        .collect();
    rows.extend(pending);

    let upserted = rows
        .iter()
        .filter(|row| {
            !state
                .rows
                .iter()
                .any(|old| old.get("sessionId") == row.get("sessionId") && old == *row)
        })
        .cloned()
        .collect();
    let removed = state
        .rows
        .iter()
        .filter_map(|row| row.get("sessionId").and_then(Value::as_str))
        .filter(|sid| {
            !rows
                .iter()
                .any(|row| row.get("sessionId").and_then(Value::as_str) == Some(*sid))
        })
        .map(str::to_owned)
        .collect();
    state.rows = rows;
    (upserted, removed)
}

fn mode_flags(mode: PermissionMode) -> (bool, bool) {
    match mode {
        PermissionMode::Ask => (false, false),
        PermissionMode::Auto => (false, true),
        PermissionMode::AlwaysApprove => (true, false),
    }
}

/// Consume agent messages; when the child dies, reconnect with backoff.
async fn run(weak: Weak<Inner>, mut rx: mpsc::UnboundedReceiver<Value>) {
    let mut refresh = tokio::time::interval(REFRESH_INTERVAL);
    refresh.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    refresh.tick().await;
    loop {
        loop {
            tokio::select! {
                msg = rx.recv() => match msg {
                    Some(msg) => {
                        let Some(inner) = weak.upgrade() else {
                            return;
                        };
                        inner.apply(&msg);
                    }
                    None => break,
                },
                _ = refresh.tick() => {
                    let Some(inner) = weak.upgrade() else {
                        return;
                    };
                    if let Err(e) = inner.refresh().await {
                        tracing::debug!(error = %e, "session list refresh failed");
                    }
                }
            }
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
    let res = handle
        .request("_x.ai/sessions/list", json!({"includeSubagents": true}))
        .await?;
    roster_rows(&res)
}

fn roster_rows(res: &Value) -> anyhow::Result<Vec<Value>> {
    res.get("result")
        .unwrap_or(res)
        .get("sessions")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| anyhow!("session roster response has no sessions array"))
}

async fn load(handle: &AgentHandle, session_id: &str, cwd: &Path) -> anyhow::Result<()> {
    handle
        .request(
            "session/load",
            json!({
                "sessionId": session_id,
                "cwd": cwd.to_string_lossy(),
                "mcpServers": [],
                "_meta": {"yoloMode": false, "autoMode": false, "noReplay": true, "postudioHistoryPaging": true},
            }),
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
    fn roster_envelope_and_mode_refresh() {
        let row = json!({"sessionId":"s", "yolo":false, "autoMode":false});
        let mut state = State::default();
        let rows = roster_rows(&json!({"result":{"sessions":[row.clone()]}})).unwrap();
        assert_eq!(replace_rows(&mut state, rows).0, vec![row.clone()]);
        assert!(replace_rows(&mut state, vec![row.clone()]).0.is_empty());
        let changed = json!({"sessionId":"s", "yolo":false, "autoMode":true});
        assert_eq!(
            replace_rows(&mut state, vec![changed.clone()]).0,
            vec![changed]
        );
        assert!(roster_rows(&json!({"invalid":true})).is_err());
        assert_eq!(
            roster_rows(&json!({"sessions":[row.clone()]})).unwrap(),
            vec![row]
        );
    }

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
