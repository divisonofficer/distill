//! ACP traffic to Studio events. Pure: no I/O, no clock.
//! Public API: plan/research-base/CONTRACT.md §3.

use serde::Deserialize;
use serde_json::Value;

use crate::attachments::{content_attachment, prompt_attachments};
use crate::sep::{Attachment, PermissionOption, QueueEntry, Sep, Status, ToolContent};

/// Client requests whose responses produce events.
#[derive(Clone, PartialEq, Eq)]
enum Pending {
    Load,
    Prompt {
        prompt_id: Option<String>,
        accepted: bool,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct QueueChangedWire {
    session_id: String,
    #[serde(default)]
    entries: Vec<QueueEntry>,
    #[serde(default)]
    running_prompt_id: Option<String>,
    #[serde(default)]
    running_text: Option<String>,
    #[serde(default)]
    running_kind: Option<String>,
    #[serde(default)]
    running_combined_texts: Option<Vec<String>>,
}

pub struct Reducer {
    session_id: String,
    /// Ids of client requests still waiting for a response.
    pending: Vec<(Value, Pending)>,
    /// Whether the latest prompt already produced `turn.end`.
    turn_ended: bool,
    /// User content awaiting its live echo from clients without prompt correlation.
    pending_user_echoes: Vec<UserEcho>,
    /// Queue id currently advertised by the shell; prevents an old completion from idling a newer turn.
    active_prompt_id: Option<String>,
    queue_snapshot_pending: bool,
}

#[derive(PartialEq, Eq)]
enum UserEcho {
    Text(String),
    Attachment(Attachment),
}

impl Reducer {
    pub fn new(session_id: String) -> Self {
        Self {
            session_id,
            pending: Vec::new(),
            turn_ended: true,
            pending_user_echoes: Vec::new(),
            active_prompt_id: None,
            queue_snapshot_pending: false,
        }
    }

    /// client to agent line. Responses from the client are not events.
    pub fn on_client(&mut self, msg: &Value) -> Vec<Sep> {
        if self.is_other_session(msg) {
            return Vec::new();
        }
        let kind = match msg.get("method").and_then(Value::as_str) {
            Some("session/prompt") => {
                let prompt_id = msg
                    .pointer("/params/_meta/promptId")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                Pending::Prompt {
                    prompt_id,
                    accepted: false,
                }
            }
            Some("session/load") => Pending::Load,
            _ => return Vec::new(),
        };
        if let Some(id) = request_id(msg) {
            self.pending.push((id, kind.clone()));
        }
        match kind {
            Pending::Prompt {
                prompt_id: Some(_), ..
            } => Vec::new(),
            Pending::Prompt {
                prompt_id: None, ..
            } => {
                self.turn_ended = false;
                let mut events = vec![Sep::SessionState {
                    status: Status::Running,
                }];
                if let Some(text) = prompt_text(msg) {
                    self.pending_user_echoes.push(UserEcho::Text(text.clone()));
                    events.push(Sep::UserMessage { text });
                }
                if let Some(blocks) = msg.pointer("/params/prompt").and_then(Value::as_array)
                    && let Ok(attachments) = prompt_attachments(blocks)
                {
                    for attachment in attachments {
                        self.pending_user_echoes
                            .push(UserEcho::Attachment(attachment.clone()));
                        events.push(Sep::UserAttachment { attachment });
                    }
                }
                events
            }
            Pending::Load => Vec::new(),
        }
    }

    /// agent to client line.
    pub fn on_agent(&mut self, msg: &Value) -> Vec<Sep> {
        if let Some((method, params)) = extension_notification(msg)
            && method.strip_prefix('_').unwrap_or(method) == "x.ai/queue/changed"
        {
            return self.queue_changed(params);
        }
        if self.is_other_session(msg) {
            return Vec::new();
        }
        match msg.get("method").and_then(Value::as_str) {
            Some("session/request_permission") => permission_request(msg),
            // `_x.ai/session/update` carries the replayed turn_completed.
            Some("session/update" | "_x.ai/session_notification" | "_x.ai/session/update") => {
                match msg.pointer("/params/update") {
                    Some(update) => self.update(update),
                    None => Vec::new(),
                }
            }
            Some(_) => Vec::new(),
            None => self.response(msg),
        }
    }

    fn is_other_session(&self, msg: &Value) -> bool {
        match msg
            .pointer("/params/sessionId")
            .or_else(|| msg.pointer("/params/params/sessionId"))
        {
            None | Some(Value::Null) => false,
            Some(id) => id.as_str() != Some(self.session_id.as_str()),
        }
    }

    pub fn expect_queue_snapshot(&mut self) {
        self.queue_snapshot_pending = true;
    }

    fn queue_changed(&mut self, params: &Value) -> Vec<Sep> {
        let Ok(queue) = serde_json::from_value::<QueueChangedWire>(params.clone()) else {
            return Vec::new();
        };
        if queue.session_id != self.session_id {
            return Vec::new();
        }
        let snapshot = std::mem::take(&mut self.queue_snapshot_pending);
        let mut accepted_ids: Vec<&str> = queue
            .entries
            .iter()
            .map(|entry| entry.id.as_str())
            .collect();
        if let Some(running_id) = queue.running_prompt_id.as_deref() {
            accepted_ids.push(running_id);
        }
        if snapshot {
            self.active_prompt_id = queue.running_prompt_id.clone();
            self.turn_ended = queue.running_prompt_id.is_none();
        } else if let Some(running_id) = queue.running_prompt_id.as_deref() {
            self.active_prompt_id = Some(running_id.to_owned());
            self.turn_ended = false;
        }
        let mut accepted_ids_to_emit = Vec::new();
        for (_, pending) in &mut self.pending {
            if let Pending::Prompt {
                prompt_id: Some(prompt_id),
                accepted,
            } = pending
                && accepted_ids.contains(&prompt_id.as_str())
            {
                if !*accepted {
                    accepted_ids_to_emit.push(prompt_id.clone());
                }
                *accepted = true;
            }
        }
        let mut events = vec![Sep::QueueChanged {
            entries: queue.entries,
            snapshot,
            running_prompt_id: queue.running_prompt_id.clone(),
            running_text: queue.running_text,
            running_kind: queue.running_kind,
            running_combined_texts: queue.running_combined_texts,
        }];
        events.extend(
            accepted_ids_to_emit
                .into_iter()
                .map(|prompt_id| Sep::PromptAccepted { prompt_id }),
        );
        if queue.running_prompt_id.is_some() {
            events.push(Sep::SessionState {
                status: Status::Running,
            });
        }
        events
    }

    fn update(&mut self, update: &Value) -> Vec<Sep> {
        let Some(kind) = str_field(update, "sessionUpdate") else {
            return Vec::new();
        };
        match kind {
            "user_message_chunk" => {
                let content = update.get("content").unwrap_or(&Value::Null);
                if hidden_from_scrollback(update, content) {
                    return Vec::new();
                }
                if let Some(text) = text_of(update) {
                    if self.consume_echo(&UserEcho::Text(text.to_owned())) {
                        Vec::new()
                    } else {
                        vec![Sep::UserMessage {
                            text: text.to_owned(),
                        }]
                    }
                } else if let Some(attachment) = content_attachment(content) {
                    if self.consume_echo(&UserEcho::Attachment(attachment.clone())) {
                        Vec::new()
                    } else {
                        vec![Sep::UserAttachment { attachment }]
                    }
                } else {
                    Vec::new()
                }
            }
            "agent_message_chunk" => text_of(update)
                .map(|text| Sep::MessageDelta {
                    text: text.to_owned(),
                })
                .into_iter()
                .collect(),
            "agent_thought_chunk" => text_of(update)
                .map(|text| Sep::ReasoningDelta {
                    text: text.to_owned(),
                })
                .into_iter()
                .collect(),
            "tool_call" => {
                let Some(id) = str_field(update, "toolCallId") else {
                    return Vec::new();
                };
                let meta = update.get("_meta").and_then(|m| m.get("x.ai/tool"));
                vec![Sep::Tool {
                    id: id.to_owned(),
                    name: meta.and_then(|m| str_field(m, "name")).map(str::to_owned),
                    title: str_field(update, "title").map(str::to_owned),
                    kind: meta
                        .and_then(|m| str_field(m, "kind"))
                        .or_else(|| str_field(update, "kind"))
                        .map(str::to_owned),
                    status: Some(status_of(update).unwrap_or_else(|| "pending".to_owned())),
                    input: update.get("rawInput").cloned(),
                    content: content_of(update),
                }]
            }
            "tool_call_update" => {
                let Some(id) = str_field(update, "toolCallId") else {
                    return Vec::new();
                };
                let status = status_of(update);
                let content = content_of(update);
                if status.is_none() && content.is_none() {
                    return Vec::new();
                }
                vec![Sep::Tool {
                    id: id.to_owned(),
                    name: None,
                    title: None,
                    kind: None,
                    status,
                    input: None,
                    content,
                }]
            }
            "auto_compact_started" => match (
                update.get("tokens_used").and_then(Value::as_u64),
                update.get("context_window").and_then(Value::as_u64),
                str_field(update, "reason"),
            ) {
                (Some(tokens_used), Some(context_window), Some(reason)) => {
                    vec![Sep::CompactionStarted {
                        tokens_used,
                        context_window,
                        reason: reason.to_owned(),
                    }]
                }
                _ => Vec::new(),
            },
            "auto_compact_completed" => {
                let tokens_before = match update.get("tokens_before") {
                    None | Some(Value::Null) => None,
                    Some(value) => match value.as_u64() {
                        Some(tokens) => Some(tokens),
                        None => return Vec::new(),
                    },
                };
                let Some(tokens_after) = update.get("tokens_after").and_then(Value::as_u64) else {
                    return Vec::new();
                };
                let elapsed_ms = match update.get("elapsed_ms") {
                    None | Some(Value::Null) => None,
                    Some(value) => match value.as_i64() {
                        Some(ms) if ms >= 0 => Some(ms),
                        _ => return Vec::new(),
                    },
                };
                vec![Sep::CompactionCompleted {
                    tokens_before,
                    tokens_after,
                    elapsed_ms,
                }]
            }
            "auto_compact_failed" => str_field(update, "error")
                .map(|error| {
                    vec![Sep::CompactionFailed {
                        error: error.to_owned(),
                    }]
                })
                .unwrap_or_default(),
            "auto_compact_cancelled" => str_field(update, "reason")
                .map(|reason| {
                    vec![Sep::CompactionCancelled {
                        reason: reason.to_owned(),
                    }]
                })
                .unwrap_or_default(),
            "usage_update" => {
                match (
                    update.get("used").and_then(Value::as_u64),
                    update.get("size").and_then(Value::as_u64),
                ) {
                    (Some(used), Some(size)) => vec![Sep::Usage { used, size }],
                    _ => Vec::new(),
                }
            }
            "session_info_update" => str_field(update, "title")
                .map(|title| {
                    vec![Sep::SessionTitle {
                        title: title.to_owned(),
                    }]
                })
                .unwrap_or_default(),
            "interaction_resolved" => match str_field(update, "tool_call_id") {
                Some(tool_call_id) => vec![
                    Sep::PermissionResolved {
                        tool_call_id: tool_call_id.to_owned(),
                    },
                    Sep::SessionState {
                        status: Status::Running,
                    },
                ],
                None => Vec::new(),
            },
            "turn_completed" => {
                let Some(stop_reason) = str_field(update, "stop_reason") else {
                    return Vec::new();
                };
                let prompt_id = update
                    .get("promptId")
                    .or_else(|| update.get("prompt_id"))
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                if self.active_prompt_id.is_some()
                    && self.active_prompt_id.as_deref() != prompt_id.as_deref()
                {
                    return Vec::new();
                }
                self.active_prompt_id = None;
                self.turn_ended = true;
                self.pending_user_echoes.clear();
                let is_error = stop_reason == "error";
                let error = if is_error {
                    str_field(update, "agent_result").map(str::to_owned)
                } else {
                    None
                };
                let status = if is_error {
                    Status::Error
                } else {
                    Status::Idle
                };
                vec![
                    Sep::TurnEnd {
                        stop_reason: stop_reason.to_owned(),
                        prompt_id,
                        error,
                    },
                    Sep::SessionState { status },
                ]
            }
            _ => Vec::new(),
        }
    }

    fn consume_echo(&mut self, echo: &UserEcho) -> bool {
        let Some(index) = self
            .pending_user_echoes
            .iter()
            .position(|pending| user_echo_matches(pending, echo))
        else {
            return false;
        };
        self.pending_user_echoes.remove(index);
        true
    }

    fn response(&mut self, msg: &Value) -> Vec<Sep> {
        let Some(id) = request_id(msg) else {
            return Vec::new();
        };
        let Some(pos) = self
            .pending
            .iter()
            .position(|(pending_id, _)| *pending_id == id)
        else {
            return Vec::new();
        };
        let (_, kind) = self.pending.remove(pos);
        match kind {
            Pending::Load => vec![Sep::ReplayEnd],
            Pending::Prompt {
                prompt_id,
                accepted,
            } => {
                let Some(error) = msg.get("error") else {
                    return prompt_id
                        .filter(|_| !accepted)
                        .map(|prompt_id| vec![Sep::PromptAccepted { prompt_id }])
                        .unwrap_or_default();
                };
                let message = error
                    .pointer("/data/message")
                    .or_else(|| error.get("message"))
                    .and_then(Value::as_str)
                    .unwrap_or("Prompt was rejected")
                    .to_owned();
                if let Some(prompt_id) = prompt_id {
                    if !accepted {
                        return vec![Sep::PromptRejected { prompt_id, message }];
                    }
                    if self.active_prompt_id.as_deref() != Some(prompt_id.as_str())
                        || self.turn_ended
                    {
                        return Vec::new();
                    }
                    self.turn_ended = true;
                    self.active_prompt_id = None;
                    return vec![
                        Sep::TurnEnd {
                            stop_reason: "error".to_owned(),
                            prompt_id: Some(prompt_id),
                            error: Some(message),
                        },
                        Sep::SessionState {
                            status: Status::Error,
                        },
                    ];
                }
                self.pending_user_echoes.clear();
                if self.turn_ended {
                    return Vec::new();
                }
                self.turn_ended = true;
                vec![
                    Sep::TurnEnd {
                        stop_reason: "error".to_owned(),
                        prompt_id: None,
                        error: Some(message),
                    },
                    Sep::SessionState {
                        status: Status::Error,
                    },
                ]
            }
        }
    }
}

fn prompt_text(msg: &Value) -> Option<String> {
    let blocks = msg.pointer("/params/prompt")?.as_array()?;
    let mut text = String::new();
    let mut found = false;
    for block in blocks {
        if str_field(block, "type") == Some("text")
            && let Some(part) = str_field(block, "text")
        {
            text.push_str(part);
            found = true;
        }
    }
    found.then_some(text)
}

fn permission_request(msg: &Value) -> Vec<Sep> {
    let (Some(id), Some(tool_call)) = (request_id(msg), msg.pointer("/params/toolCall")) else {
        return Vec::new();
    };
    let Some(tool_call_id) = str_field(tool_call, "toolCallId") else {
        return Vec::new();
    };
    let options = msg
        .pointer("/params/options")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(permission_option).collect())
        .unwrap_or_default();
    vec![
        Sep::PermissionRequest {
            id,
            tool_call_id: tool_call_id.to_owned(),
            title: str_field(tool_call, "title").unwrap_or_default().to_owned(),
            options,
        },
        Sep::SessionState {
            status: Status::WaitingPermission,
        },
    ]
}

fn permission_option(option: &Value) -> Option<PermissionOption> {
    Some(PermissionOption {
        id: str_field(option, "optionId")?.to_owned(),
        kind: str_field(option, "kind")?.to_owned(),
        name: str_field(option, "name")?.to_owned(),
    })
}

fn content_of(update: &Value) -> Option<Vec<ToolContent>> {
    update
        .get("content")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(tool_content).collect())
}

fn tool_content(item: &Value) -> Option<ToolContent> {
    match str_field(item, "type")? {
        "diff" => Some(ToolContent::Diff {
            path: str_field(item, "path")?.to_owned(),
            old: str_field(item, "oldText").map(str::to_owned),
            new: str_field(item, "newText")?.to_owned(),
        }),
        "content" => {
            let inner = item.get("content")?;
            match str_field(inner, "type")? {
                "text" => Some(ToolContent::Text {
                    text: str_field(inner, "text")?.to_owned(),
                }),
                "image" => Some(ToolContent::Image {
                    mime: str_field(inner, "mimeType")?.to_owned(),
                    data: str_field(inner, "data")?.to_owned(),
                }),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Lowercased tool status; `inprogress` and `in_progress` both map to `in_progress`.
fn status_of(update: &Value) -> Option<String> {
    let status = str_field(update, "status")?.to_lowercase();
    if matches!(status.as_str(), "inprogress" | "in_progress") {
        Some("in_progress".to_owned())
    } else {
        Some(status)
    }
}

fn user_echo_matches(left: &UserEcho, right: &UserEcho) -> bool {
    match (left, right) {
        (UserEcho::Text(left), UserEcho::Text(right)) => left == right,
        (
            UserEcho::Attachment(Attachment::Image {
                mime: left_mime,
                data: left_data,
                ..
            }),
            UserEcho::Attachment(Attachment::Image {
                mime: right_mime,
                data: right_data,
                ..
            }),
        ) => left_mime == right_mime && left_data == right_data,
        (UserEcho::Attachment(left), UserEcho::Attachment(right)) => left == right,
        _ => false,
    }
}

fn hidden_from_scrollback(update: &Value, content: &Value) -> bool {
    [update, content].into_iter().any(|value| {
        ["_meta", "meta"].into_iter().any(|key| {
            value
                .get(key)
                .and_then(|meta| meta.get("hideFromScrollback"))
                .and_then(Value::as_bool)
                == Some(true)
        })
    })
}

fn text_of(update: &Value) -> Option<&str> {
    let content = update.get("content")?;
    if str_field(content, "type")? != "text" {
        return None;
    }
    str_field(content, "text")
}

fn str_field<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key)?.as_str()
}

/// JSON-RPC id of a request or response. `null` counts as absent.
fn extension_notification(msg: &Value) -> Option<(&str, &Value)> {
    let outer = msg.get("method")?.as_str()?;
    let params = msg.get("params")?;
    if matches!(
        outer,
        "x.ai/session_notification" | "_x.ai/session_notification"
    ) {
        let method = params.get("method")?.as_str()?;
        let payload = params.get("params")?;
        Some((method, payload))
    } else {
        Some((outer, params))
    }
}

fn request_id(msg: &Value) -> Option<Value> {
    msg.get("id").filter(|id| !id.is_null()).cloned()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const SID: &str = "s1";

    fn reducer() -> Reducer {
        Reducer::new(SID.to_owned())
    }

    #[test]
    fn other_session_message_is_dropped() {
        let mut r = reducer();
        let msg = json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s2",
            "update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"x"}}}});
        assert!(r.on_agent(&msg).is_empty());
        let prompt = json!({"id":1,"jsonrpc":"2.0","method":"session/prompt",
            "params":{"sessionId":"s2","prompt":[]}});
        assert!(r.on_client(&prompt).is_empty());
    }

    #[test]
    fn prompt_error_without_turn_completed_ends_turn() {
        let mut r = reducer();
        let prompt = json!({"id":7,"jsonrpc":"2.0","method":"session/prompt",
            "params":{"sessionId":SID,"prompt":[]}});
        assert_eq!(
            r.on_client(&prompt),
            vec![Sep::SessionState {
                status: Status::Running
            }]
        );
        let err = json!({"id":7,"jsonrpc":"2.0","error":{"code":-32603,
            "message":"Internal error","data":{"message":"boom"}}});
        assert_eq!(
            r.on_agent(&err),
            vec![
                Sep::TurnEnd {
                    stop_reason: "error".into(),
                    prompt_id: None,
                    error: Some("boom".into())
                },
                Sep::SessionState {
                    status: Status::Error
                },
            ]
        );
    }

    #[test]
    fn prompt_error_after_turn_completed_is_silent() {
        let mut r = reducer();
        let prompt = json!({"id":8,"jsonrpc":"2.0","method":"session/prompt",
            "params":{"sessionId":SID,"prompt":[]}});
        r.on_client(&prompt);
        let done = json!({"jsonrpc":"2.0","method":"_x.ai/session_notification","params":{
            "sessionId":SID,"update":{"sessionUpdate":"turn_completed","stop_reason":"error",
            "agent_result":"bad"}}});
        assert_eq!(r.on_agent(&done).len(), 2);
        let err =
            json!({"id":8,"jsonrpc":"2.0","error":{"code":-32603,"message":"Internal error"}});
        assert!(r.on_agent(&err).is_empty());
    }

    #[test]
    fn correlated_prompt_waits_for_shell_acceptance_and_rejection_restores_identity() {
        let mut r = reducer();
        let prompt = json!({"id":7,"jsonrpc":"2.0","method":"session/prompt",
            "params":{"sessionId":SID,"prompt":[],"_meta":{"promptId":"p7"}}});
        assert!(r.on_client(&prompt).is_empty());
        let err = json!({"id":7,"jsonrpc":"2.0","error":{"code":-32603,"message":"blocked"}});
        assert_eq!(
            r.on_agent(&err),
            vec![Sep::PromptRejected {
                prompt_id: "p7".into(),
                message: "blocked".into(),
            }]
        );
    }

    #[test]
    fn queue_changed_accepts_correlated_prompts_and_drops_stale_turn_results() {
        let mut r = reducer();
        let prompt = json!({"id":9,"jsonrpc":"2.0","method":"session/prompt",
            "params":{"sessionId":SID,"prompt":[],"_meta":{"promptId":"p2"}}});
        assert!(r.on_client(&prompt).is_empty());
        let queue = json!({"jsonrpc":"2.0","method":"_x.ai/queue/changed","params":{
            "sessionId":SID,"entries":[{"id":"p2","version":3,"owner":"postudio-node",
            "lastEditor":null,"kind":"prompt","text":"next","position":0}],
            "runningPromptId":"p1","runningText":"current","runningKind":"prompt"}});
        assert_eq!(
            r.on_agent(&queue),
            vec![
                Sep::QueueChanged {
                    entries: vec![QueueEntry {
                        id: "p2".into(),
                        version: 3,
                        owner: Some("postudio-node".into()),
                        last_editor: None,
                        kind: "prompt".into(),
                        text: "next".into(),
                        position: 0,
                        combined_texts: None,
                    }],
                    snapshot: false,
                    running_prompt_id: Some("p1".into()),
                    running_text: Some("current".into()),
                    running_kind: Some("prompt".into()),
                    running_combined_texts: None,
                },
                Sep::PromptAccepted {
                    prompt_id: "p2".into(),
                },
                Sep::SessionState {
                    status: Status::Running
                },
            ]
        );
        let promoted = json!({"method":"x.ai/queue/changed","params":{
            "sessionId":SID,"entries":[],"runningPromptId":"p2"}});
        assert_eq!(
            r.on_agent(&promoted),
            vec![
                Sep::QueueChanged {
                    entries: vec![],
                    snapshot: false,
                    running_prompt_id: Some("p2".into()),
                    running_text: None,
                    running_kind: None,
                    running_combined_texts: None,
                },
                Sep::SessionState {
                    status: Status::Running
                },
            ]
        );
        let stale = json!({"method":"_x.ai/session_notification","params":{"sessionId":SID,
            "update":{"sessionUpdate":"turn_completed","promptId":"p1","stop_reason":"end_turn"}}});
        assert!(r.on_agent(&stale).is_empty());
        let current = json!({"method":"_x.ai/session_notification","params":{"sessionId":SID,
            "update":{"sessionUpdate":"turn_completed","promptId":"p2","stop_reason":"end_turn"}}});
        assert!(
            matches!(r.on_agent(&current).as_slice(), [Sep::TurnEnd { prompt_id: Some(pid), .. }, Sep::SessionState { status: Status::Idle }] if pid == "p2")
        );
        let accepted_error = json!({"id":9,"jsonrpc":"2.0","error":{"message":"late failure"}});
        assert!(r.on_agent(&accepted_error).is_empty());
    }

    #[test]
    fn successful_send_now_goal_steering_ack_does_not_idle_running_turn() {
        let mut r = reducer();
        let running = json!({"method":"x.ai/queue/changed","params":{
            "sessionId":SID,"entries":[],"runningPromptId":"goal-turn"}});
        assert!(r.on_agent(&running).iter().any(|event| matches!(
            event,
            Sep::SessionState {
                status: Status::Running
            }
        )));
        let prompt = json!({"id":11,"jsonrpc":"2.0","method":"session/prompt",
            "params":{"sessionId":SID,"prompt":[],"_meta":{"promptId":"steer-11","sendNow":true}}});
        assert!(r.on_client(&prompt).is_empty());
        let success = json!({"id":11,"jsonrpc":"2.0","result":{"stopReason":"cancelled"}});
        assert_eq!(
            r.on_agent(&success),
            vec![Sep::PromptAccepted {
                prompt_id: "steer-11".into()
            }]
        );
        assert_eq!(r.active_prompt_id.as_deref(), Some("goal-turn"));
        assert!(!r.turn_ended);
    }

    #[test]
    fn explicit_queue_snapshot_replaces_stale_running_prompt() {
        let mut r = reducer();
        r.active_prompt_id = Some("finished-while-disconnected".into());
        r.turn_ended = false;
        r.expect_queue_snapshot();
        let queue = json!({"method":"x.ai/queue/changed","params":{"sessionId":SID,"entries":[]}});
        assert_eq!(
            r.on_agent(&queue),
            vec![Sep::QueueChanged {
                entries: vec![],
                snapshot: true,
                running_prompt_id: None,
                running_text: None,
                running_kind: None,
                running_combined_texts: None,
            }]
        );
        assert_eq!(r.active_prompt_id, None);
        assert!(r.turn_ended);
    }

    #[test]
    fn queue_changed_is_session_scoped() {
        let mut r = reducer();
        let foreign =
            json!({"method":"x.ai/queue/changed","params":{"sessionId":"s2","entries":[]}});
        assert!(r.on_agent(&foreign).is_empty());
    }

    #[test]
    fn permission_request_waits() {
        let mut r = reducer();
        let req = json!({"id":3,"jsonrpc":"2.0","method":"session/request_permission","params":{
            "sessionId":SID,"options":[{"optionId":"allow-once","kind":"allow_once","name":"Yes"}],
            "toolCall":{"toolCallId":"c1","title":"Edit `a`"}}});
        assert_eq!(
            r.on_agent(&req),
            vec![
                Sep::PermissionRequest {
                    id: json!(3),
                    tool_call_id: "c1".into(),
                    title: "Edit `a`".into(),
                    options: vec![PermissionOption {
                        id: "allow-once".into(),
                        kind: "allow_once".into(),
                        name: "Yes".into(),
                    }],
                },
                Sep::SessionState {
                    status: Status::WaitingPermission
                },
            ]
        );
    }

    #[test]
    fn interaction_resolved_runs() {
        let mut r = reducer();
        let msg = json!({"jsonrpc":"2.0","method":"_x.ai/session_notification","params":{
            "sessionId":SID,"update":{"sessionUpdate":"interaction_resolved","tool_call_id":"c1"}}});
        assert_eq!(
            r.on_agent(&msg),
            vec![
                Sep::PermissionResolved {
                    tool_call_id: "c1".into()
                },
                Sep::SessionState {
                    status: Status::Running
                },
            ]
        );
    }

    #[test]
    fn compaction_lifecycle_preserves_fields_and_validates_payloads() {
        let mut r = reducer();
        let started = json!({"method":"_x.ai/session_notification","params":{"sessionId":SID,"update":{"sessionUpdate":"auto_compact_started","tokens_used":820,"context_window":1000,"percentage":82,"reason":"threshold"}}});
        assert_eq!(
            r.on_agent(&started),
            vec![Sep::CompactionStarted {
                tokens_used: 820,
                context_window: 1000,
                reason: "threshold".into(),
            }]
        );

        let completed = json!({"method":"_x.ai/session_notification","params":{"sessionId":SID,"update":{"sessionUpdate":"auto_compact_completed","tokens_before":820,"tokens_after":210,"elapsed_ms":1500,"summary_preview":"ignored"}}});
        assert_eq!(
            r.on_agent(&completed),
            vec![Sep::CompactionCompleted {
                tokens_before: Some(820),
                tokens_after: 210,
                elapsed_ms: Some(1500),
            }]
        );
        for optional_fields in [json!({}), json!({"tokens_before":null,"elapsed_ms":null})] {
            let msg = json!({"method":"session/update","params":{"sessionId":SID,"update":{
                "sessionUpdate":"auto_compact_completed","tokens_after":210,
                "tokens_before":optional_fields.get("tokens_before"),
                "elapsed_ms":optional_fields.get("elapsed_ms")
            }}});
            assert_eq!(
                r.on_agent(&msg),
                vec![Sep::CompactionCompleted {
                    tokens_before: None,
                    tokens_after: 210,
                    elapsed_ms: None,
                }]
            );
        }
        for (before, elapsed) in [
            (json!("bad"), json!(1)),
            (json!(1), json!("bad")),
            (json!(1), json!(-1)),
        ] {
            let msg = json!({"method":"session/update","params":{"sessionId":SID,"update":{
                "sessionUpdate":"auto_compact_completed","tokens_after":210,
                "tokens_before":before,"elapsed_ms":elapsed
            }}});
            assert!(r.on_agent(&msg).is_empty());
        }
        for (kind, payload) in [
            ("auto_compact_failed", json!({"error":"oom"})),
            ("auto_compact_cancelled", json!({"reason":"user_cancelled"})),
        ] {
            let msg = json!({"method":"session/update","params":{"sessionId":SID,"update":{
                "sessionUpdate":kind,"error":payload.get("error"),"reason":payload.get("reason")
            }}});
            assert_eq!(
                r.on_agent(&msg),
                vec![if kind == "auto_compact_failed" {
                    Sep::CompactionFailed {
                        error: "oom".into(),
                    }
                } else {
                    Sep::CompactionCancelled {
                        reason: "user_cancelled".into(),
                    }
                }]
            );
            let foreign = json!({"method":"_x.ai/session_notification","params":{"sessionId":"s2","update":{
                "sessionUpdate":kind,"error":payload.get("error"),"reason":payload.get("reason")
            }}});
            assert!(r.on_agent(&foreign).is_empty());
        }
        for update in [
            json!({"sessionUpdate":"auto_compact_started","tokens_used":"820","context_window":1000,"reason":"threshold"}),
            json!({"sessionUpdate":"auto_compact_started","tokens_used":820,"context_window":-1,"reason":"threshold"}),
            json!({"sessionUpdate":"auto_compact_started","tokens_used":820,"context_window":1000,"reason":7}),
            json!({"sessionUpdate":"auto_compact_completed","tokens_after":"210"}),
            json!({"sessionUpdate":"auto_compact_failed","error":false}),
            json!({"sessionUpdate":"auto_compact_cancelled","reason":false}),
        ] {
            let msg = json!({"method":"session/update","params":{"sessionId":SID,"update":update}});
            assert!(r.on_agent(&msg).is_empty());
        }
    }

    #[test]
    fn status_strings_are_normalized() {
        let mut r = reducer();
        let msg = json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":SID,
            "update":{"sessionUpdate":"tool_call_update","toolCallId":"c1","status":"InProgress"}}});
        assert_eq!(
            r.on_agent(&msg),
            vec![Sep::Tool {
                id: "c1".into(),
                name: None,
                title: None,
                kind: None,
                status: Some("in_progress".into()),
                input: None,
                content: None,
            }]
        );
    }
}
