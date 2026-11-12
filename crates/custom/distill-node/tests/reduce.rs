use std::fs;
use std::path::{Path, PathBuf};

use distill_node::reduce::Reducer;
use distill_node::sep::{Attachment, Sep, Status, ToolContent};
use serde_json::{Value, json};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// Replay one `acp/*.jsonl` fixture through a reducer and collect the events.
fn run(path: &Path) -> Vec<Sep> {
    let text = fs::read_to_string(path).unwrap();
    let lines: Vec<(String, Value)> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let row: Value = serde_json::from_str(line).unwrap();
            (row["dir"].as_str().unwrap().to_owned(), row["msg"].clone())
        })
        .collect();
    // Session id: the loaded one if the fixture loads, else the first `session/new` result.
    let session_id = lines
        .iter()
        .find(|(_, msg)| msg["method"] == "session/load")
        .or_else(|| {
            lines
                .iter()
                .find(|(_, msg)| msg["result"]["sessionId"].is_string())
        })
        .map(|(_, msg)| {
            msg["params"]["sessionId"]
                .as_str()
                .or_else(|| msg["result"]["sessionId"].as_str())
                .unwrap()
                .to_owned()
        })
        .unwrap();
    let mut reducer = Reducer::new(session_id);
    let mut out = Vec::new();
    for (dir, msg) in &lines {
        match dir.as_str() {
            "c2a" => out.extend(reducer.on_client(msg)),
            "a2c" => out.extend(reducer.on_agent(msg)),
            other => panic!("unknown dir {other}"),
        }
    }
    out
}

#[test]
fn acp_fixtures_match_sep_expectations() {
    let acp_dir = fixtures().join("acp");
    let mut count = 0;
    for entry in fs::read_dir(&acp_dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
        let expected_path = fixtures().join("sep").join(format!("{stem}.json"));
        let expected: Vec<Sep> = serde_json::from_str(
            &fs::read_to_string(&expected_path)
                .unwrap_or_else(|e| panic!("{}: {e}", expected_path.display())),
        )
        .unwrap();
        assert_eq!(run(&path), expected, "fixture {stem}");
        count += 1;
    }
    assert_eq!(count, 7);
}

#[test]
fn replayed_tool_call_keeps_status_kind_and_output() {
    let mut reducer = Reducer::new("s1".into());
    let events = reducer.on_agent(&json!({
        "method": "session/update",
        "params": {
            "sessionId": "s1",
            "update": {
                "sessionUpdate": "tool_call",
                "toolCallId": "c1",
                "title": "Read file",
                "kind": "read",
                "status": "Completed",
                "rawInput": {"path": "file.txt"},
                "content": [{
                    "type": "content",
                    "content": {"type": "text", "text": "file output"}
                }]
            }
        }
    }));
    assert_eq!(
        events,
        vec![Sep::Tool {
            id: "c1".into(),
            name: None,
            title: Some("Read file".into()),
            kind: Some("read".into()),
            status: Some("completed".into()),
            input: Some(json!({"path": "file.txt"})),
            content: Some(vec![ToolContent::Text {
                text: "file output".into()
            }]),
        }]
    );
}

#[test]
fn outgoing_prompt_emits_user_message_and_suppresses_echo() {
    let mut reducer = Reducer::new("s1".into());
    let prompt = json!({
        "id": 1,
        "method": "session/prompt",
        "params": {
            "sessionId": "s1",
            "prompt": [
                {"type": "text", "text": "hello "},
                {"type": "image", "data": "ignored"},
                {"type": "text", "text": "world"}
            ]
        }
    });
    assert_eq!(
        reducer.on_client(&prompt),
        vec![
            Sep::SessionState {
                status: Status::Running
            },
            Sep::UserMessage {
                text: "hello world".into()
            }
        ]
    );

    let echo = json!({
        "method": "session/update",
        "params": {
            "sessionId": "s1",
            "update": {
                "sessionUpdate": "user_message_chunk",
                "content": {"type": "text", "text": "hello world"}
            }
        }
    });
    assert!(reducer.on_agent(&echo).is_empty());
}

#[test]
fn same_prompt_text_in_distinct_turns_emits_once_per_turn() {
    let mut reducer = Reducer::new("s1".into());
    for id in [1, 2] {
        let prompt = json!({
            "id": id,
            "method": "session/prompt",
            "params": {
                "sessionId": "s1",
                "prompt": [{"type": "text", "text": "repeat"}]
            }
        });
        assert_eq!(
            reducer.on_client(&prompt),
            vec![
                Sep::SessionState {
                    status: Status::Running
                },
                Sep::UserMessage {
                    text: "repeat".into()
                }
            ]
        );
        let echo = json!({
            "method": "session/update",
            "params": {
                "sessionId": "s1",
                "update": {
                    "sessionUpdate": "user_message_chunk",
                    "content": {"type": "text", "text": "repeat"}
                }
            }
        });
        assert!(reducer.on_agent(&echo).is_empty());
        let completed = json!({
            "method": "session/update",
            "params": {
                "sessionId": "s1",
                "update": {"sessionUpdate": "turn_completed", "stop_reason": "end_turn"}
            }
        });
        assert_eq!(
            reducer.on_agent(&completed),
            vec![
                Sep::TurnEnd {
                    stop_reason: "end_turn".into(),
                    error: None
                },
                Sep::SessionState {
                    status: Status::Idle
                }
            ]
        );
    }
}

#[test]
fn foreign_session_prompt_and_echo_are_filtered() {
    let mut reducer = Reducer::new("s1".into());
    let foreign_prompt = json!({
        "id": 1,
        "method": "session/prompt",
        "params": {
            "sessionId": "s2",
            "prompt": [{"type": "text", "text": "local"}]
        }
    });
    assert!(reducer.on_client(&foreign_prompt).is_empty());

    let prompt = json!({
        "id": 2,
        "method": "session/prompt",
        "params": {
            "sessionId": "s1",
            "prompt": [{"type": "text", "text": "local"}]
        }
    });
    assert_eq!(
        reducer.on_client(&prompt),
        vec![
            Sep::SessionState {
                status: Status::Running
            },
            Sep::UserMessage {
                text: "local".into()
            }
        ]
    );
    let foreign_echo = json!({
        "method": "session/update",
        "params": {
            "sessionId": "s2",
            "update": {
                "sessionUpdate": "user_message_chunk",
                "content": {"type": "text", "text": "local"}
            }
        }
    });
    assert!(reducer.on_agent(&foreign_echo).is_empty());
    let own_echo = json!({
        "method": "session/update",
        "params": {
            "sessionId": "s1",
            "update": {
                "sessionUpdate": "user_message_chunk",
                "content": {"type": "text", "text": "local"}
            }
        }
    });
    assert!(reducer.on_agent(&own_echo).is_empty());
}

#[test]
fn replayed_inline_image_and_resource_text_become_attachments() {
    let mut reducer = Reducer::new("s1".into());
    let image = reducer.on_agent(&json!({
        "method": "session/update",
        "params": {"sessionId":"s1", "update": {
            "sessionUpdate":"user_message_chunk",
            "content":{"type":"image", "mimeType":"image/png", "data":"aGVsbG8="}
        }}
    }));
    assert_eq!(
        image,
        vec![Sep::UserAttachment {
            attachment: Attachment::Image {
                mime: "image/png".into(),
                data: "aGVsbG8=".into(),
                name: None,
            }
        }]
    );

    let resource = reducer.on_agent(&json!({
        "method": "session/update",
        "params": {"sessionId":"s1", "update": {
            "sessionUpdate":"user_message_chunk",
            "content":{"type":"resource", "resource": {
                "uri":"file:///tmp/notes.md", "mimeType":"text/markdown", "text":"saved notes"
            }}
        }}
    }));
    assert_eq!(
        resource,
        vec![Sep::UserAttachment {
            attachment: Attachment::Text {
                name: "notes.md".into(),
                text: "saved notes".into(),
            }
        }]
    );
}

#[test]
fn prompt_resource_and_resource_link_map_to_attachments() {
    let mut reducer = Reducer::new("s1".into());
    let events = reducer.on_client(&json!({
        "id": 5,
        "method": "session/prompt",
        "params": {"sessionId":"s1", "prompt":[
            {"type":"resource", "resource":{"uri":"file:///attachments/a.txt", "text":"A"}},
            {"type":"resource_link", "name":"b.txt", "uri":"https://example.test/b.txt"}
        ]}
    }));
    assert_eq!(
        events,
        vec![
            Sep::SessionState {
                status: Status::Running
            },
            Sep::UserAttachment {
                attachment: Attachment::Text {
                    name: "a.txt".into(),
                    text: "A".into()
                }
            },
            Sep::UserAttachment {
                attachment: Attachment::File {
                    name: "b.txt".into(),
                    uri: Some("https://example.test/b.txt".into())
                }
            },
        ]
    );
}

#[test]
fn foreign_and_hidden_user_attachments_are_ignored() {
    let mut reducer = Reducer::new("s1".into());
    for message in [
        json!({"method":"session/update", "params":{"sessionId":"s2", "update":{
            "sessionUpdate":"user_message_chunk", "content":{"type":"image", "mimeType":"image/png", "data":"aGVsbG8="}
        }}}),
        json!({"method":"session/update", "params":{"sessionId":"s1", "update":{
            "sessionUpdate":"user_message_chunk", "_meta":{"hideFromScrollback":true},
            "content":{"type":"image", "mimeType":"image/png", "data":"aGVsbG8="}
        }}}),
        json!({"method":"session/update", "params":{"sessionId":"s1", "update":{
            "sessionUpdate":"user_message_chunk",
            "content":{"type":"text", "text":"internal", "_meta":{"hideFromScrollback":true}}
        }}}),
    ] {
        assert!(reducer.on_agent(&message).is_empty());
    }
}

#[test]
fn repeated_attachment_submissions_consume_one_matching_echo_each() {
    let mut reducer = Reducer::new("s1".into());
    let prompt = json!({"id":1, "method":"session/prompt", "params":{
        "sessionId":"s1", "prompt":[{"type":"image", "mimeType":"image/png", "data":"aGVsbG8="}]
    }});
    let echo = json!({"method":"session/update", "params":{"sessionId":"s1", "update":{
        "sessionUpdate":"user_message_chunk",
        "content":{"type":"image", "mimeType":"image/png", "data":"aGVsbG8="}
    }}});
    for id in [1, 2] {
        let mut submitted = prompt.clone();
        submitted["id"] = json!(id);
        assert_eq!(reducer.on_client(&submitted).len(), 2);
    }
    assert!(reducer.on_agent(&echo).is_empty());
    assert!(reducer.on_agent(&echo).is_empty());
    assert_eq!(
        reducer.on_agent(&echo),
        vec![Sep::UserAttachment {
            attachment: Attachment::Image {
                mime: "image/png".into(),
                data: "aGVsbG8=".into(),
                name: None,
            }
        }]
    );
}
