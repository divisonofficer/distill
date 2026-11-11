//! Session WebSocket: one agent child per client, `session/load` replay, then live SEP events.
//! Public API: plan/research-base/CONTRACT.md §4.

use std::path::PathBuf;

use axum::Router;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::response::Response;
use axum::routing::get;
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::task::JoinSet;

use crate::agent::{self, AgentConfig, AgentHandle};
use crate::keeper::Keeper;
use crate::reduce::Reducer;
use crate::sep::{ClientMsg, PermissionMode, Sep};

#[derive(Clone)]
struct Ctx {
    cfg: AgentConfig,
    keeper: Keeper,
}

pub fn routes(cfg: AgentConfig, keeper: Keeper) -> Router {
    Router::new()
        .route("/ws/session/{id}", get(ws_session))
        .with_state(Ctx { cfg, keeper })
}

async fn ws_session(
    State(ctx): State<Ctx>,
    Path(id): Path<String>,
    ws: WebSocketUpgrade,
) -> Response {
    ws.on_upgrade(move |socket| run(socket, ctx, id))
}

async fn run(mut socket: WebSocket, ctx: Ctx, id: String) {
    let Some(cwd) = lookup_cwd(&ctx.keeper, &id).await else {
        emit(&mut socket, &[error("unknown session")]).await;
        return;
    };
    let (handle, mut inbound) = match open(&ctx.cfg).await {
        Ok(opened) => opened,
        Err(e) => {
            emit(&mut socket, &[error(e)]).await;
            return;
        }
    };
    let mut conn = Conn::new(id.clone(), handle);
    let (_, events) = conn.request(
        "session/load",
        json!({"sessionId": id, "cwd": cwd.to_string_lossy(), "mcpServers": []}),
    );
    if !emit(&mut socket, &events).await {
        return;
    }
    loop {
        tokio::select! {
            // Biased: agent lines queued before a response are applied before that response.
            biased;
            msg = inbound.recv() => {
                let Some(msg) = msg else { break };
                if !emit(&mut socket, &conn.on_agent_msg(&msg)).await {
                    break;
                }
            }
            Some(done) = conn.tasks.join_next(), if !conn.tasks.is_empty() => {
                let Ok(done) = done else { continue };
                match conn.on_done(done) {
                    Ok(events) => {
                        if !emit(&mut socket, &events).await {
                            break;
                        }
                    }
                    Err(e) => {
                        emit(&mut socket, &[error(e)]).await;
                        break;
                    }
                }
            }
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Text(text))) => {
                    if !emit(&mut socket, &conn.on_client_msg(&text)).await {
                        break;
                    }
                }
                Some(Ok(Message::Close(_)) | Err(_)) | None => break,
                Some(Ok(_)) => {}
            }
        }
    }
    let _ = socket.send(Message::Close(None)).await;
}

/// Client requests and notifications go to the child; responses come back through `tasks`.
struct Conn {
    id: String,
    handle: AgentHandle,
    reducer: Reducer,
    tasks: JoinSet<Done>,
    next_id: u64,
    warned_unsupported: bool,
}

struct Done {
    id: u64,
    method: &'static str,
    result: anyhow::Result<Value>,
}

impl Conn {
    fn new(id: String, handle: AgentHandle) -> Self {
        Self {
            reducer: Reducer::new(id.clone()),
            id,
            handle,
            tasks: JoinSet::new(),
            next_id: 0,
            warned_unsupported: false,
        }
    }

    /// Returns the synthetic request id and the events the reducer makes from the request.
    fn request(&mut self, method: &'static str, params: Value) -> (u64, Vec<Sep>) {
        self.next_id += 1;
        let id = self.next_id;
        let events = self.reducer.on_client(
            &json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params.clone()}),
        );
        let handle = self.handle.clone();
        self.tasks.spawn(async move {
            let result = handle.request(method, params).await;
            Done { id, method, result }
        });
        (id, events)
    }

    fn notify(&mut self, method: &str, params: Value) -> Vec<Sep> {
        let mut events = self
            .reducer
            .on_client(&json!({"jsonrpc": "2.0", "method": method, "params": params.clone()}));
        if let Err(e) = self.handle.notify(method, params) {
            events.push(error(e));
        }
        events
    }

    fn respond(&mut self, id: Value, result: Value) -> Vec<Sep> {
        let mut events = self
            .reducer
            .on_client(&json!({"jsonrpc": "2.0", "id": id.clone(), "result": result.clone()}));
        if let Err(e) = self.handle.respond(id, result) {
            events.push(error(e));
        }
        events
    }

    fn on_client_msg(&mut self, raw: &str) -> Vec<Sep> {
        let msg: ClientMsg = match serde_json::from_str(raw) {
            Ok(msg) => msg,
            Err(e) => return vec![error(format!("bad message: {e}"))],
        };
        let sid = self.id.clone();
        match msg {
            ClientMsg::Prompt { text, images } => {
                let mut blocks = vec![json!({"type": "text", "text": text})];
                blocks.extend(
                    images.into_iter().map(
                        |img| json!({"type": "image", "mimeType": img.mime, "data": img.data}),
                    ),
                );
                self.request(
                    "session/prompt",
                    json!({"sessionId": sid, "prompt": blocks}),
                )
                .1
            }
            ClientMsg::Cancel => self.notify("session/cancel", json!({"sessionId": sid})),
            ClientMsg::Permission { id, option_id } => self.respond(
                id,
                json!({"outcome": {"outcome": "selected", "optionId": option_id}}),
            ),
            ClientMsg::Mode { mode } => {
                self.notify("_x.ai/yolo_mode_changed", mode_params(&sid, mode))
            }
        }
    }

    fn on_agent_msg(&mut self, msg: &Value) -> Vec<Sep> {
        if !self.warned_unsupported
            && msg.get("id").is_some()
            && let Some(method) = msg.get("method").and_then(Value::as_str)
            && method != "session/request_permission"
        {
            self.warned_unsupported = true;
            tracing::warn!(method, "agent request not supported; ignored");
        }
        self.reducer.on_agent(msg)
    }

    /// Err when the session/load failed: the caller closes the socket.
    fn on_done(&mut self, done: Done) -> anyhow::Result<Vec<Sep>> {
        let response = match done.result {
            Ok(result) => json!({"jsonrpc": "2.0", "id": done.id, "result": result}),
            Err(e) if done.method == "session/load" => return Err(e),
            // The turn outlasts the request timeout; turn_completed still ends it.
            Err(e)
                if done.method == "session/prompt" && e.to_string().contains("no response in") =>
            {
                tracing::debug!(error = %e, "prompt still running");
                return Ok(Vec::new());
            }
            Err(e) => json!({"jsonrpc": "2.0", "id": done.id, "error": {"message": e.to_string()}}),
        };
        Ok(self.reducer.on_agent(&response))
    }
}

fn mode_params(sid: &str, mode: PermissionMode) -> Value {
    match mode {
        PermissionMode::Ask => json!({
            "sessionId": sid, "yolo_mode": false, "auto_mode": false, "permission_mode": "ask"
        }),
        PermissionMode::Auto => json!({
            "sessionId": sid, "auto_mode": true, "permission_mode": "auto"
        }),
        PermissionMode::AlwaysApprove => json!({"sessionId": sid, "yolo_mode": true}),
    }
}

async fn open(cfg: &AgentConfig) -> anyhow::Result<(AgentHandle, UnboundedReceiver<Value>)> {
    let (handle, inbound) = agent::spawn(cfg).await?;
    handle.initialize().await?;
    Ok((handle, inbound))
}

async fn lookup_cwd(keeper: &Keeper, id: &str) -> Option<PathBuf> {
    if let Some(cwd) = keeper.cwd_of(id) {
        return Some(cwd);
    }
    keeper.refresh().await.ok()?;
    keeper.cwd_of(id)
}

fn error(message: impl std::fmt::Display) -> Sep {
    Sep::Error {
        message: message.to_string(),
    }
}

/// False once the socket is gone.
async fn emit(socket: &mut WebSocket, events: &[Sep]) -> bool {
    for event in events {
        let Ok(text) = serde_json::to_string(event) else {
            continue;
        };
        if socket.send(Message::Text(text.into())).await.is_err() {
            return false;
        }
    }
    true
}
