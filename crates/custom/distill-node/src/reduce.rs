//! ACP traffic to Studio events. Pure: no I/O, no clock.
//! Public API: plan/research-base/CONTRACT.md §3.

use serde_json::Value;

use crate::sep::{PermissionOption, Sep, Status, ToolContent};

/// Client requests whose responses produce events.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pending {
    Load,
    Prompt,
}

pub struct Reducer {
    session_id: String,
    /// Ids of client requests still waiting for a response.
    pending: Vec<(Value, Pending)>,
    /// Whether the latest prompt already produced `turn.end`.
    turn_ended: bool,
    /// Text sent by the latest prompt, awaiting its echoed user update.
    pending_user_echo: Option<String>,
}

impl Reducer {
    pub fn new(session_id: String) -> Self {
        Self {
            session_id,
            pending: Vec::new(),
            turn_ended: true,
            pending_user_echo: None,
        }
    }

    /// client to agent line. Responses from the client are not events.
    pub fn on_client(&mut self, msg: &Value) -> Vec<Sep> {
        if self.is_other_session(msg) {
            return Vec::new();
        }
        let kind = match msg.get("method").and_then(Value::as_str) {
            Some("session/prompt") => Pending::Prompt,
            Some("session/load") => Pending::Load,
            _ => return Vec::new(),
        };
        if let Some(id) = request_id(msg) {
            self.pending.push((id, kind));
        }
        match kind {
            Pending::Prompt => {
                self.turn_ended = false;
                self.pending_user_echo = prompt_text(msg);
                let mut events = vec![Sep::SessionState {
                    status: Status::Running,
                }];
                if let Some(text) = &self.pending_user_echo {
                    events.push(Sep::UserMessage { text: text.clone() });
                }
                events
            }
            Pending::Load => Vec::new(),
        }
    }

    /// agent to client line.
    pub fn on_agent(&mut self, msg: &Value) -> Vec<Sep> {
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
        match msg.pointer("/params/sessionId") {
            None | Some(Value::Null) => false,
            Some(id) => id.as_str() != Some(self.session_id.as_str()),
        }
    }

    fn update(&mut self, update: &Value) -> Vec<Sep> {
        let Some(kind) = str_field(update, "sessionUpdate") else {
            return Vec::new();
        };
        match kind {
            "user_message_chunk" => {
                let Some(text) = text_of(update) else {
                    return Vec::new();
                };
                if self.pending_user_echo.as_deref() == Some(text) {
                    self.pending_user_echo = None;
                    Vec::new()
                } else {
                    vec![Sep::UserMessage {
                        text: text.to_owned(),
                    }]
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
                self.turn_ended = true;
                self.pending_user_echo = None;
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
                        error,
                    },
                    Sep::SessionState { status },
                ]
            }
            _ => Vec::new(),
        }
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
            Pending::Prompt => {
                let Some(error) = msg.get("error") else {
                    return Vec::new();
                };
                self.pending_user_echo = None;
                if self.turn_ended {
                    return Vec::new();
                }
                self.turn_ended = true;
                let message = error
                    .pointer("/data/message")
                    .or_else(|| error.get("message"))
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                vec![
                    Sep::TurnEnd {
                        stop_reason: "error".to_owned(),
                        error: message,
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
