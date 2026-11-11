//! Studio Event Protocol (SEP): what the node sends to web clients and accepts from them.
//! Field meanings and the ACP mapping are in plan/research-base/CONTRACT.md §2-§3.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Running,
    WaitingPermission,
    Idle,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolContent {
    Text {
        text: String,
    },
    /// `data` is base64.
    Image {
        mime: String,
        data: String,
    },
    Diff {
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        old: Option<String>,
        new: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionOption {
    pub id: String,
    /// `allow_once`, `allow_always` or `reject_once`.
    pub kind: String,
    pub name: String,
}

/// Node to client.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t")]
pub enum Sep {
    #[serde(rename = "session.state")]
    SessionState { status: Status },
    #[serde(rename = "session.title")]
    SessionTitle { title: String },
    #[serde(rename = "user.message")]
    UserMessage { text: String },
    #[serde(rename = "message.delta")]
    MessageDelta { text: String },
    #[serde(rename = "reasoning.delta")]
    ReasoningDelta { text: String },
    /// Upsert keyed by `id`: a field that is present replaces the client's stored value.
    #[serde(rename = "tool")]
    Tool {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kind: Option<String>,
        /// `pending`, `in_progress`, `completed` or `failed`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        input: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        content: Option<Vec<ToolContent>>,
    },
    /// `id` is the JSON-RPC id of the agent's request; echo it in [`ClientMsg::Permission`].
    #[serde(rename = "permission.request")]
    PermissionRequest {
        id: Value,
        tool_call_id: String,
        title: String,
        options: Vec<PermissionOption>,
    },
    #[serde(rename = "permission.resolved")]
    PermissionResolved { tool_call_id: String },
    #[serde(rename = "usage")]
    Usage { used: u64, size: u64 },
    #[serde(rename = "turn.end")]
    TurnEnd {
        stop_reason: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    /// History replay after attaching is complete; later events are live.
    #[serde(rename = "replay.end")]
    ReplayEnd,
    #[serde(rename = "error")]
    Error { message: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PermissionMode {
    Ask,
    Auto,
    AlwaysApprove,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageIn {
    pub mime: String,
    /// base64
    pub data: String,
}

/// Client to node, on the session WebSocket.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ClientMsg {
    Prompt {
        text: String,
        #[serde(default)]
        images: Vec<ImageIn>,
    },
    Cancel,
    Permission {
        id: Value,
        option_id: String,
    },
    Mode {
        mode: PermissionMode,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_names_are_stable() {
        let tool = Sep::Tool {
            id: "c1".into(),
            name: None,
            title: None,
            kind: None,
            status: Some("completed".into()),
            input: None,
            content: Some(vec![ToolContent::Diff {
                path: "a.txt".into(),
                old: None,
                new: "x".into(),
            }]),
        };
        assert_eq!(
            serde_json::to_string(&tool).unwrap(),
            r#"{"t":"tool","id":"c1","status":"completed","content":[{"type":"diff","path":"a.txt","new":"x"}]}"#
        );
        assert_eq!(
            serde_json::to_string(&Sep::SessionState {
                status: Status::WaitingPermission
            })
            .unwrap(),
            r#"{"t":"session.state","status":"waiting_permission"}"#
        );
        assert_eq!(
            serde_json::to_string(&Sep::ReplayEnd).unwrap(),
            r#"{"t":"replay.end"}"#
        );
        let msg: ClientMsg =
            serde_json::from_str(r#"{"t":"mode","mode":"always-approve"}"#).unwrap();
        assert_eq!(
            msg,
            ClientMsg::Mode {
                mode: PermissionMode::AlwaysApprove
            }
        );
        let msg: ClientMsg = serde_json::from_str(r#"{"t":"prompt","text":"hi"}"#).unwrap();
        assert_eq!(
            msg,
            ClientMsg::Prompt {
                text: "hi".into(),
                images: vec![]
            }
        );
    }
}
