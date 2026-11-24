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
use crate::sep::{ClientMsg, PermissionMode, Sep, Status};

const POSTUDIO_CLIENT_IDENTIFIER: &str = "postudio-node";
const QUEUE_SNAPSHOT_PREFIX: &str = "postudio-queue-snapshot-";
const MAX_PROMPT_ID_BYTES: usize = 128;

#[derive(Clone)]
struct Ctx {
    cfg: AgentConfig,
    keeper: Keeper,
    research: Option<crate::research_context::ResearchContext>,
}

pub fn routes(cfg: AgentConfig, keeper: Keeper) -> Router {
    routes_inner(cfg, keeper, None)
}

pub(crate) fn routes_with_context(
    cfg: AgentConfig,
    keeper: Keeper,
    research: crate::research_context::ResearchContext,
) -> Router {
    routes_inner(cfg, keeper, Some(research))
}

fn routes_inner(
    cfg: AgentConfig,
    keeper: Keeper,
    research: Option<crate::research_context::ResearchContext>,
) -> Router {
    Router::new()
        .route("/ws/session/{id}", get(ws_session))
        .with_state(Ctx {
            cfg,
            keeper,
            research,
        })
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
    let mut conn = Conn::new(
        id.clone(),
        handle,
        cwd.to_string_lossy().into_owned(),
        ctx.research.clone(),
    );
    let (_, events) = conn.request(
        "session/load",
        json!({
            "sessionId": id,
            "cwd": cwd.to_string_lossy(),
            "mcpServers": ctx.keeper.mcp_servers(&id),
            "_meta": {"yoloMode": false, "autoMode": false, "noReplay": true, "postudioHistoryPaging": true},
        }),
    );
    if !emit(&mut socket, &events).await {
        return;
    }
    let mut inbound_open = true;
    loop {
        tokio::select! {
            // Biased: agent lines queued before a response are applied before that response.
            biased;
            msg = inbound.recv(), if inbound_open => {
                let Some(msg) = msg else {
                    inbound_open = false;
                    if conn.tasks.is_empty() { break; }
                    continue;
                };
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
                if !inbound_open && conn.tasks.is_empty() { break; }
            }
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Text(text))) => {
                    if !emit(&mut socket, &conn.on_client_msg(&text).await).await {
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
    model_id: Option<String>,
    runtime_status: Status,
    history_cwd: String,
    history_cursor: Option<String>,
    history_has_more: bool,
    history_request_pending: bool,
    research: Option<crate::research_context::ResearchContext>,
}

struct Done {
    id: u64,
    method: &'static str,
    result: anyhow::Result<Value>,
}

impl Conn {
    fn new(
        id: String,
        handle: AgentHandle,
        history_cwd: String,
        research: Option<crate::research_context::ResearchContext>,
    ) -> Self {
        Self {
            reducer: Reducer::new(id.clone()),
            id,
            handle,
            tasks: JoinSet::new(),
            next_id: 0,
            interactions: Outstanding::default(),
            permissions: Vec::new(),
            model_id: None,
            runtime_status: Status::Idle,
            history_cwd,
            history_cursor: None,
            history_has_more: false,
            history_request_pending: false,
            research,
        }
    }

    /// Returns the synthetic request id and the events the reducer makes from the request.
    fn request(&mut self, method: &'static str, params: Value) -> (u64, Vec<Sep>) {
        self.next_id += 1;
        let id = self.next_id;
        let events = self.reducer.on_client(
            &json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params.clone()}),
        );
        self.record_runtime_status(&events);
        let handle = self.handle.clone();
        self.tasks.spawn(async move {
            let result = if method == "session/prompt" {
                handle.request_without_timeout(method, params).await
            } else {
                handle.request(method, params).await
            };
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

    async fn on_client_msg(&mut self, raw: &str) -> Vec<Sep> {
        let msg: ClientMsg = match serde_json::from_str(raw) {
            Ok(msg) => msg,
            Err(e) => return vec![error(format!("bad message: {e}"))],
        };
        let sid = self.id.clone();
        match msg {
            ClientMsg::HistoryLoad { cursor } => {
                if self.history_request_pending
                    || !self.history_has_more
                    || self.history_cursor.as_deref() != Some(cursor.as_str())
                {
                    return vec![history_page_error(
                        self.history_cursor.clone(),
                        self.history_has_more,
                        false,
                        "History cursor is stale",
                    )];
                }
                self.request_history(Some(cursor), false);
                Vec::new()
            }
            ClientMsg::Prompt {
                text,
                images,
                text_files,
                prompt_id,
                send_now,
            } => {
                if prompt_id
                    .as_deref()
                    .is_some_and(|id| id.trim().is_empty() || id.len() > MAX_PROMPT_ID_BYTES)
                {
                    return prompt_rejected(
                        prompt_id,
                        "prompt_id must be a non-empty string of at most 128 bytes",
                    );
                }
                let mut blocks =
                    match crate::attachments::prompt_blocks(Some(&text), &images, &text_files) {
                        Ok(blocks) => blocks,
                        Err(message) => return prompt_rejected(prompt_id, message),
                    };
                if let Some(research) = &self.research {
                    match research.blocks(&sid).await {
                        Ok(mut context) => blocks.append(&mut context),
                        Err(message) => return prompt_rejected(prompt_id, message),
                    }
                }
                self.request(
                    "session/prompt",
                    prompt_params(&sid, &blocks, prompt_id.as_deref(), send_now),
                )
                .1
            }
            ClientMsg::QueueRemove {
                id,
                expected_version,
            } => {
                if id.trim().is_empty() || id.len() > MAX_PROMPT_ID_BYTES {
                    return vec![error(
                        "queue id must be a non-empty string of at most 128 bytes",
                    )];
                }
                let snapshot_probe = id.starts_with(QUEUE_SNAPSHOT_PREFIX);
                let events = self.notify(
                    "_x.ai/queue/remove",
                    queue_remove_params(&sid, &id, expected_version),
                );
                if snapshot_probe
                    && !events
                        .iter()
                        .any(|event| matches!(event, Sep::Error { .. }))
                {
                    self.reducer.expect_queue_snapshot();
                }
                events
            }
            ClientMsg::Cancel => self.notify("session/cancel", json!({"sessionId": sid})),
            ClientMsg::Permission { id, option_id } => {
                let Some(index) = self
                    .permissions
                    .iter()
                    .position(|(pending, _, _)| *pending == id)
                else {
                    if self.interactions.contains_id(&id) {
                        return vec![error("unknown permission request or option")];
                    }
                    return vec![Sep::PermissionExpired { id }];
                };
                let (_, tool_call_id, options) = &self.permissions[index];
                if !options.contains(&option_id) {
                    return vec![error("unknown permission request or option")];
                }
                let tool_call_id = tool_call_id.clone();
                if let Err(e) = self.handle.respond(
                    id,
                    json!({"outcome": {"outcome": "selected", "optionId": option_id}}),
                ) {
                    return vec![error(e)];
                }
                self.permissions.remove(index);
                self.runtime_status = Status::Running;
                vec![Sep::PermissionResolved { tool_call_id }]
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
            ClientMsg::Model { model_id } => {
                if model_id.trim().is_empty() {
                    return vec![error("model_id must not be empty")];
                }
                self.request(
                    "session/set_model",
                    json!({"sessionId": sid, "modelId": model_id}),
                )
                .1
            }
        }
    }

    fn on_agent_msg(&mut self, msg: &Value) -> Vec<Sep> {
        if is_historical_replay(msg) {
            let events = self.reducer.on_agent(msg);
            let state = events
                .iter()
                .filter(|event| matches!(event, Sep::SessionTitle { .. } | Sep::Usage { .. }))
                .cloned()
                .collect();
            return state;
        }
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
        if let Some(model_id) = model_changed(msg, &self.id) {
            return self.select_model(model_id, false);
        }
        let events = self.reducer.on_agent(msg);
        self.record_runtime_status(&events);
        for event in &events {
            match event {
                Sep::PermissionRequest {
                    id,
                    tool_call_id,
                    options,
                    ..
                } => {
                    self.permissions
                        .retain(|(pending, tool, _)| pending != id && tool != tool_call_id);
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

    fn record_runtime_status(&mut self, events: &[Sep]) {
        if let Some(status) = events.iter().rev().find_map(|event| match event {
            Sep::SessionState { status } => Some(*status),
            _ => None,
        }) {
            self.runtime_status = status;
        }
    }

    fn select_model(&mut self, model_id: String, force: bool) -> Vec<Sep> {
        if !force && self.model_id.as_deref() == Some(model_id.as_str()) {
            return Vec::new();
        }
        self.model_id = Some(model_id.clone());
        vec![Sep::SessionModel { model_id }]
    }

    fn add_interaction(&mut self, request: Interaction) -> Vec<Sep> {
        if let Err(message) = self.interactions.insert(request.clone()) {
            return self.respond_error(request.id().clone(), -32602, &message);
        }
        self.runtime_status = Status::WaitingPermission;
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

    fn request_history(&mut self, cursor: Option<String>, initial: bool) {
        self.history_request_pending = true;
        self.next_id += 1;
        let id = self.next_id;
        let handle = self.handle.clone();
        let session_id = self.id.clone();
        let cwd = self.history_cwd.clone();
        self.tasks.spawn(async move {
            let result = handle
                .request_without_timeout(
                    "_x.ai/session/load_history",
                    json!({"sessionId": session_id, "cwd": cwd, "cursor": cursor}),
                )
                .await;
            Done {
                id,
                method: if initial {
                    "history.initial"
                } else {
                    "history.older"
                },
                result,
            }
        });
    }

    /// Err when the session/load failed: the caller closes the socket.
    fn on_done(&mut self, done: Done) -> anyhow::Result<Vec<Sep>> {
        let result = match done.result {
            Ok(result) => result,
            Err(e) if done.method == "session/load" => return Err(e),
            Err(e) if done.method == "session/set_model" => return Ok(vec![error(e)]),
            Err(e) if done.method == "history.initial" || done.method == "history.older" => {
                self.history_request_pending = false;
                let initial = done.method == "history.initial";
                let mut events = vec![history_page_error(
                    self.history_cursor.clone(),
                    self.history_has_more,
                    initial,
                    &e.to_string(),
                )];
                if initial {
                    events.push(Sep::SessionState {
                        status: self.runtime_status,
                    });
                    events.push(Sep::ReplayEnd);
                }
                return Ok(events);
            }
            Err(e) => {
                let response =
                    json!({"jsonrpc":"2.0", "id":done.id, "error":{"message":e.to_string()}});
                let events = self.reducer.on_agent(&response);
                self.record_runtime_status(&events);
                return Ok(events);
            }
        };
        let response = json!({"jsonrpc": "2.0", "id": done.id, "result": result});
        let mut events = self.reducer.on_agent(&response);
        match done.method {
            "session/load" => {
                if let Some(model_id) = loaded_model_id(&result) {
                    let mut selected = self.select_model(model_id, false);
                    selected.extend(events);
                    events = selected;
                }
                events.retain(|event| !matches!(event, Sep::ReplayEnd));
                self.request_history(None, true);
            }
            "history.initial" | "history.older" => {
                self.history_request_pending = false;
                let initial = done.method == "history.initial";
                let items = result
                    .get("items")
                    .cloned()
                    .and_then(|value| serde_json::from_value::<Vec<Sep>>(value).ok());
                let Some(items) = items else {
                    let mut events = vec![history_page_error(
                        self.history_cursor.clone(),
                        self.history_has_more,
                        initial,
                        "Invalid history page from agent",
                    )];
                    if initial {
                        events.push(Sep::SessionState {
                            status: self.runtime_status,
                        });
                        events.push(Sep::ReplayEnd);
                    }
                    return Ok(events);
                };
                let cursor = result
                    .get("cursor")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let has_more = result
                    .get("has_more")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                self.history_cursor = cursor.clone();
                self.history_has_more = has_more;
                events.clear();
                if initial {
                    if let Some(title) = result.get("title").and_then(Value::as_str) {
                        events.push(Sep::SessionTitle {
                            title: title.to_owned(),
                        });
                    }
                    if let (Some(used), Some(size)) = (
                        result.pointer("/usage/used").and_then(Value::as_u64),
                        result.pointer("/usage/size").and_then(Value::as_u64),
                    ) {
                        events.push(Sep::Usage { used, size });
                    }
                }
                events.push(Sep::HistoryPage {
                    items,
                    cursor,
                    has_more,
                    initial,
                    error: None,
                });
                if initial {
                    events.push(Sep::SessionState {
                        status: self.runtime_status,
                    });
                    events.push(Sep::ReplayEnd);
                }
            }
            "session/set_model" => {
                if let Some(model_id) = canonical_model_id(&result) {
                    events.extend(self.select_model(model_id, true));
                }
            }
            _ => {}
        }
        Ok(events)
    }
}

fn prompt_rejected(prompt_id: Option<String>, message: impl Into<String>) -> Vec<Sep> {
    let message = message.into();
    match prompt_id {
        Some(prompt_id) => vec![Sep::PromptRejected { prompt_id, message }],
        None => vec![error(message)],
    }
}

fn prompt_params(
    session_id: &str,
    blocks: &[Value],
    prompt_id: Option<&str>,
    send_now: bool,
) -> Value {
    let mut params = json!({"sessionId": session_id, "prompt": blocks});
    let mut meta = serde_json::Map::new();
    meta.insert(
        "clientIdentifier".to_owned(),
        Value::String(POSTUDIO_CLIENT_IDENTIFIER.to_owned()),
    );
    meta.insert("queuePolicy".to_owned(), Value::String("strict".to_owned()));
    if let Some(prompt_id) = prompt_id {
        meta.insert("promptId".to_owned(), Value::String(prompt_id.to_owned()));
    }
    if send_now {
        meta.insert("sendNow".to_owned(), Value::Bool(true));
    }
    params["_meta"] = Value::Object(meta);
    params
}

fn queue_remove_params(session_id: &str, id: &str, expected_version: u64) -> Value {
    json!({
        "sessionId": session_id,
        "id": id,
        "expectedVersion": expected_version,
        "owner": POSTUDIO_CLIENT_IDENTIFIER,
    })
}

fn model_changed(msg: &Value, expected_session: &str) -> Option<String> {
    let method = msg.get("method")?.as_str()?;
    let params = msg.get("params")?;
    let (method, params) = match (
        params.get("method").and_then(Value::as_str),
        params.get("params"),
    ) {
        (Some(inner), Some(inner_params)) => (inner, inner_params),
        _ => (method, params),
    };
    let method = method.strip_prefix('_').unwrap_or(method);
    if !matches!(method, "x.ai/session_notification" | "session/update")
        || params.get("sessionId").and_then(Value::as_str) != Some(expected_session)
    {
        return None;
    }
    let update = params.get("update")?;
    if update.get("sessionUpdate").and_then(Value::as_str) != Some("model_changed") {
        return None;
    }
    nonempty_string(update.get("model_id").or_else(|| update.get("modelId")))
}

fn loaded_model_id(result: &Value) -> Option<String> {
    let model_id = result
        .pointer("/models/currentModelId")
        .or_else(|| result.pointer("/_meta/x.ai~1sessionDetail/currentModelId"))
        .or_else(|| result.pointer("/meta/x.ai~1sessionDetail/currentModelId"))
        .or_else(|| result.pointer("/_meta/sessionDetail/currentModelId"))
        .or_else(|| result.pointer("/meta/sessionDetail/currentModelId"))
        .or_else(|| result.get("currentModelId"));
    nonempty_string(model_id)
}

fn canonical_model_id(result: &Value) -> Option<String> {
    let model_id = result
        .pointer("/_meta/canonicalModelId")
        .or_else(|| result.pointer("/meta/canonicalModelId"))
        .or_else(|| result.get("canonicalModelId"))
        .or_else(|| result.pointer("/_meta/model/modelId"))
        .or_else(|| result.pointer("/_meta/model/id"))
        .or_else(|| result.pointer("/meta/model/modelId"))
        .or_else(|| result.pointer("/meta/model/id"))
        .or_else(|| result.pointer("/model/modelId"))
        .or_else(|| result.pointer("/model/id"));
    nonempty_string(model_id)
}

fn nonempty_string(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
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
    if keeper.cwd_of(id).is_none() {
        keeper.refresh().await.ok()?;
    }
    let cwd = keeper.cwd_of(id)?;
    keeper.attach(id).await.ok()?;
    Some(cwd)
}

fn history_page_error(cursor: Option<String>, has_more: bool, initial: bool, message: &str) -> Sep {
    Sep::HistoryPage {
        items: Vec::new(),
        cursor,
        has_more,
        initial,
        error: Some(message.to_owned()),
    }
}

fn is_historical_replay(msg: &Value) -> bool {
    matches!(
        msg.get("method").and_then(Value::as_str),
        Some("session/update" | "_x.ai/session/update" | "_x.ai/session_notification")
    ) && (msg
        .pointer("/params/_meta/isReplay")
        .and_then(Value::as_bool)
        == Some(true)
        || msg
            .pointer("/params/params/_meta/isReplay")
            .and_then(Value::as_bool)
            == Some(true))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejected_correlated_prompts_keep_submission_identity() {
        assert_eq!(
            prompt_rejected(Some("p1".into()), "bad attachment"),
            vec![Sep::PromptRejected {
                prompt_id: "p1".into(),
                message: "bad attachment".into(),
            }]
        );
        assert!(matches!(
            prompt_rejected(None, "bad attachment").as_slice(),
            [Sep::Error { .. }]
        ));
    }

    #[test]
    fn prompt_params_use_supported_queue_and_send_now_metadata() {
        let block = json!({"type":"text","text":"next"});
        let queued = prompt_params("s1", std::slice::from_ref(&block), Some("p1"), false);
        assert_eq!(queued["_meta"]["promptId"], "p1");
        assert_eq!(
            queued["_meta"]["clientIdentifier"],
            POSTUDIO_CLIENT_IDENTIFIER
        );
        assert_eq!(queued["_meta"]["queuePolicy"], "strict");
        assert_eq!(queued["_meta"].get("sendNow"), None);
        assert_eq!(queued["sessionId"], "s1");
        let send_now = prompt_params("s1", &[block], Some("p2"), true);
        assert_eq!(send_now["_meta"]["sendNow"], true);
        assert_eq!(
            queue_remove_params("s1", "p1", 4),
            json!({"sessionId":"s1","id":"p1","expectedVersion":4,"owner":"postudio-node"})
        );
    }

    #[test]
    fn only_tagged_session_updates_are_buffered_as_history() {
        assert!(is_historical_replay(&json!({
            "method": "session/update",
            "params": {"_meta": {"isReplay": true}, "update": {}}
        })));
        assert!(!is_historical_replay(&json!({
            "method": "session/update",
            "params": {"update": {}}
        })));
        assert!(!is_historical_replay(&json!({
            "method": "session/prompt",
            "params": {"_meta": {"isReplay": true}}
        })));
    }
}
