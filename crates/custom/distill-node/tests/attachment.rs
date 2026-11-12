use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use distill_node::attachments::prompt_blocks;
use distill_node::sep::{Attachment, ClientMsg, ImageIn, Sep, TextFileIn};
use serde_json::json;

fn image(mime: &str, data: &str) -> ImageIn {
    ImageIn {
        mime: mime.into(),
        data: data.into(),
        name: None,
    }
}

#[test]
fn prompt_images_and_text_files_become_acp_blocks() {
    let blocks = prompt_blocks(
        Some("ask"),
        &[image("image/png", "aGVsbG8=")],
        &[TextFileIn {
            name: "notes draft.md".into(),
            text: "notes".into(),
        }],
    )
    .unwrap();
    assert_eq!(
        blocks,
        vec![
            json!({"type":"text", "text":"ask"}),
            json!({"type":"image", "mimeType":"image/png", "data":"aGVsbG8="}),
            json!({
                "type":"resource",
                "resource": {
                    "uri":"file:///attachments/notes_draft.md",
                    "mimeType":"text/plain",
                    "text":"notes"
                }
            })
        ]
    );
}

#[test]
fn client_prompt_attachment_fields_default_for_old_wire_messages() {
    let msg: ClientMsg = serde_json::from_value(json!({"t":"prompt", "text":"hi"})).unwrap();
    assert_eq!(
        msg,
        ClientMsg::Prompt {
            text: "hi".into(),
            images: vec![],
            text_files: vec![],
        }
    );
}

#[test]
fn rejects_unsupported_mime_and_malformed_base64() {
    assert!(prompt_blocks(None, &[image("image/svg+xml", "aGVsbG8=")], &[]).is_err());
    assert!(prompt_blocks(None, &[image("image/png", "not base64!")], &[]).is_err());
}

#[test]
fn rejects_oversized_and_missing_name_attachments() {
    let encoded = STANDARD.encode(vec![0; 10 * 1024 * 1024 + 1]);
    assert!(prompt_blocks(None, &[image("image/png", &encoded)], &[]).is_err());
    assert!(
        prompt_blocks(
            None,
            &[],
            &[TextFileIn {
                name: "  ".into(),
                text: "x".into(),
            }]
        )
        .is_err()
    );
}

#[test]
fn rejects_attachment_count_and_total_size_limits() {
    let too_many = vec![image("image/png", "AA=="); 11];
    assert!(prompt_blocks(None, &too_many, &[]).is_err());
    let images = vec![image("image/png", &STANDARD.encode(vec![0; 3 * 1024 * 1024])); 7];
    assert!(prompt_blocks(None, &images, &[]).is_err());
    assert!(
        prompt_blocks(
            None,
            &[],
            &[TextFileIn {
                name: "large.txt".into(),
                text: "x".repeat(1024 * 1024 + 1),
            }]
        )
        .is_err()
    );
}

#[test]
fn sep_user_attachment_wire_shape_is_stable() {
    assert_eq!(
        serde_json::to_value(Sep::UserAttachment {
            attachment: Attachment::File {
                name: "report.txt".into(),
                uri: Some("https://example.test/report.txt".into()),
            },
        })
        .unwrap(),
        json!({
            "t":"user.attachment",
            "attachment":{"kind":"file", "name":"report.txt", "uri":"https://example.test/report.txt"}
        })
    );
}
