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
use crate::interaction::{self, Interaction, Outstanding};
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
        json!({
            "sessionId": id,
            "cwd": cwd.to_string_lossy(),
            "mcpServers": [],
            "_meta": {"yoloMode": false, "autoMode": false},
        }),
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
    interactions: Outstanding,
    permissions: Vec<(Value, String, Vec<String>)>,
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
            interactions: Outstanding::default(),
            permissions: Vec::new(),
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
            ClientMsg::Permission { id, option_id } => {
                let Some(index) = self.permissions.iter().position(|(pending, _, options)| {
                    *pending == id && options.contains(&option_id)
                }) else {
                    return vec![error("unknown permission request or option")];
                };
                self.permissions.remove(index);
                self.respond(
                    id,
                    json!({"outcome": {"outcome": "selected", "optionId": option_id}}),
                )
            }
            ClientMsg::QuestionReply {
                id,
                session_id,
                tool_call_id,
                outcome,
                answers,
            } => {
                match self.interactions.answer_question(
                    id,
                    &session_id,
                    &tool_call_id,
                    outcome,
                    answers,
                ) {
                    Ok((id, result)) => self.respond(id, result),
                    Err(e) => vec![error(e)],
                }
            }
            ClientMsg::PlanReply {
                id,
                session_id,
                tool_call_id,
                outcome,
                feedback,
            } => {
                match self.interactions.answer_plan(
                    id,
                    &session_id,
                    &tool_call_id,
                    outcome,
                    feedback,
                ) {
                    Ok((id, result)) => self.respond(id, result),
                    Err(e) => vec![error(e)],
                }
            }
            ClientMsg::Mode { mode } => {
                self.notify("_x.ai/yolo_mode_changed", mode_params(&sid, mode))
            }
        }
    }

    fn on_agent_msg(&mut self, msg: &Value) -> Vec<Sep> {
        if let (Some(id), Some(method)) = (msg.get("id"), msg.get("method").and_then(Value::as_str))
        {
            match interaction::parse_request(msg, &self.id) {
                Ok(Some(request)) => return self.add_interaction(request),
                Err(message) => return self.respond_error(id.clone(), -32602, &message),
                Ok(None) if method == "session/request_permission" => {
                    if let Err(message) = validate_permission_request(msg, &self.id) {
                        return self.respond_error(id.clone(), -32602, &message);
                    }
                }
                Ok(None) => {
                    tracing::warn!(method, "unsupported agent request rejected");
                    return self.respond_error(id.clone(), -32601, "Method not found");
                }
            }
        }
        let events = self.reducer.on_agent(msg);
        for event in &events {
            match event {
                Sep::PermissionRequest {
                    id,
                    tool_call_id,
                    options,
                    ..
                } => {
                    self.permissions.retain(|(pending, _, _)| pending != id);
                    self.permissions.push((
                        id.clone(),
                        tool_call_id.clone(),
                        options.iter().map(|option| option.id.clone()).collect(),
                    ));
                }
                Sep::PermissionResolved { tool_call_id } => {
                    self.permissions.retain(|(_, tool, _)| tool != tool_call_id);
                    self.interactions.remove_tool_call(tool_call_id)
                }
                Sep::TurnEnd { .. } => {
                    self.permissions.clear();
                    self.interactions.clear();
                }
                _ => {}
            }
        }
        events
    }

    fn add_interaction(&mut self, request: Interaction) -> Vec<Sep> {
        if let Err(message) = self.interactions.insert(request.clone()) {
            return self.respond_error(request.id().clone(), -32602, &message);
        }
        match request {
            Interaction::Question {
                id,
                session_id,
                tool_call_id,
                mode,
                questions,
                preview,
            } => vec![Sep::QuestionRequest {
                id,
                session_id,
                tool_call_id,
                mode,
                questions,
                preview,
            }],
            Interaction::Plan {
                id,
                session_id,
                tool_call_id,
                plan_content,
            } => vec![Sep::PlanRequest {
                id,
                session_id,
                tool_call_id,
                plan_content,
            }],
        }
    }

    fn respond_error(&mut self, id: Value, code: i64, message: &str) -> Vec<Sep> {
        self.handle
            .respond_error(id, code, message)
            .err()
            .map(error)
            .into_iter()
            .collect()
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

fn validate_permission_request(msg: &Value, expected_session: &str) -> Result<(), String> {
    let params = msg
        .get("params")
        .and_then(Value::as_object)
        .ok_or_else(|| "permission params must be an object".to_owned())?;
    if params.get("sessionId").and_then(Value::as_str) != Some(expected_session) {
        return Err("permission session does not match this connection".into());
    }
    if msg
        .pointer("/params/toolCall/toolCallId")
        .and_then(Value::as_str)
        .is_none()
    {
        return Err("permission toolCallId must be a string".into());
    }
    let options = params
        .get("options")
        .and_then(Value::as_array)
        .ok_or_else(|| "permission options must be an array".to_owned())?;
    for option in options {
        if ["optionId", "kind", "name"]
            .iter()
            .any(|field| option.get(field).and_then(Value::as_str).is_none())
        {
            return Err("permission option fields must be strings".into());
        }
    }
    Ok(())
}

fn mode_params(sid: &str, mode: PermissionMode) -> Value {
    match mode {
        PermissionMode::Ask => json!({
            "sessionId": sid, "yolo_mode": false, "auto_mode": false, "permission_mode": "ask"
        }),
        PermissionMode::Auto => json!({
            "sessionId": sid, "yolo_mode": false, "auto_mode": true, "permission_mode": "auto"
        }),
        PermissionMode::AlwaysApprove => json!({
            "sessionId": sid, "yolo_mode": true, "auto_mode": false
        }),
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
