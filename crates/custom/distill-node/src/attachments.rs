//! Attachment validation and ACP prompt conversion.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde_json::{Value, json};

use crate::sep::{Attachment, ImageIn, TextFileIn};

const MAX_ATTACHMENTS: usize = 10;
const MAX_IMAGE_BYTES: usize = 10 * 1024 * 1024;
const MAX_TEXT_BYTES: usize = 1024 * 1024;
const MAX_TOTAL_BYTES: usize = 20 * 1024 * 1024;
const MAX_IMAGE_BASE64: usize = 4 * MAX_IMAGE_BYTES.div_ceil(3);

/// Validate user attachments and build ACP prompt blocks.
pub fn prompt_blocks(
    text: Option<&str>,
    images: &[ImageIn],
    text_files: &[TextFileIn],
) -> Result<Vec<Value>, String> {
    let attachments = input_attachments(images, text_files)?;
    let mut blocks = Vec::with_capacity(attachments.len() + usize::from(text.is_some()));
    if let Some(text) = text {
        blocks.push(json!({"type": "text", "text": text}));
    }
    for attachment in attachments {
        match attachment {
            Attachment::Image { mime, data, .. } => {
                blocks.push(json!({"type": "image", "mimeType": mime, "data": data}));
            }
            Attachment::Text { name, text } => blocks.push(json!({
                "type": "resource",
                "resource": {
                    "uri": format!("file:///attachments/{name}"),
                    "mimeType": "text/plain",
                    "text": text
                }
            })),
            Attachment::File { .. } => {
                return Err("file links are not accepted as client attachments".into());
            }
        }
    }
    Ok(blocks)
}

/// Map supported ACP prompt blocks to Studio attachments.
pub fn prompt_attachments(blocks: &[Value]) -> Result<Vec<Attachment>, String> {
    let mut attachments = Vec::new();
    for block in blocks {
        match block.get("type").and_then(Value::as_str) {
            Some("image") => attachments.push(image_attachment(
                block.get("mimeType").and_then(Value::as_str),
                block.get("data").and_then(Value::as_str),
                block.get("name").and_then(Value::as_str),
            )?),
            Some("resource") => {
                let resource = block.get("resource").unwrap_or(block);
                if let Some(text) = resource.get("text").and_then(Value::as_str) {
                    let uri = resource.get("uri").and_then(Value::as_str);
                    attachments.push(Attachment::Text {
                        name: uri
                            .and_then(resource_name)
                            .ok_or("resource text has no filename")?,
                        text: checked_text(text)?,
                    });
                }
            }
            Some("resource_link") => {
                let name = safe_basename(
                    block
                        .get("name")
                        .and_then(Value::as_str)
                        .ok_or("file attachment has no filename")?,
                )?;
                let uri = block.get("uri").and_then(Value::as_str).map(str::to_owned);
                attachments.push(Attachment::File { name, uri });
            }
            _ => {}
        }
    }
    validate_attachments(&attachments)?;
    Ok(attachments)
}

/// Map a live or replayed ACP user content block to a supported attachment.
pub fn content_attachment(content: &Value) -> Option<Attachment> {
    match content.get("type")?.as_str()? {
        "image" => image_attachment(
            content.get("mimeType")?.as_str(),
            content.get("data")?.as_str(),
            content.get("name").and_then(Value::as_str),
        )
        .ok(),
        "resource" => {
            let resource = content.get("resource").unwrap_or(content);
            let text = resource.get("text")?.as_str()?;
            Some(Attachment::Text {
                name: resource_name(resource.get("uri")?.as_str()?)?,
                text: checked_text(text).ok()?,
            })
        }
        "resource_link" => Some(Attachment::File {
            name: safe_basename(content.get("name")?.as_str()?).ok()?,
            uri: content
                .get("uri")
                .and_then(Value::as_str)
                .map(str::to_owned),
        }),
        _ => None,
    }
}

/// Validate and normalize ordinary-chat text files before signing them.
pub(crate) fn chat_text_files(files: &[TextFileIn]) -> Result<Vec<TextFileIn>, String> {
    prompt_blocks(None, &[], files)?;
    if files.iter().map(|file| file.text.len()).sum::<usize>() > MAX_TEXT_BYTES {
        return Err("chat text files exceed 1 MiB total".into());
    }
    files
        .iter()
        .map(|file| {
            let name = safe_basename(&file.name)?;
            let extension = name.rsplit('.').next().unwrap_or_default();
            if !extension.eq_ignore_ascii_case("txt") && !extension.eq_ignore_ascii_case("md") {
                return Err("chat text files must use TXT or MD filenames".into());
            }
            Ok(TextFileIn {
                name,
                text: file.text.clone(),
            })
        })
        .collect()
}

fn input_attachments(
    images: &[ImageIn],
    text_files: &[TextFileIn],
) -> Result<Vec<Attachment>, String> {
    if images.len() + text_files.len() > MAX_ATTACHMENTS {
        return Err("too many attachments (maximum 10)".into());
    }
    let mut attachments = Vec::with_capacity(images.len() + text_files.len());
    for image in images {
        attachments.push(image_attachment(
            Some(&image.mime),
            Some(&image.data),
            image.name.as_deref(),
        )?);
    }
    for file in text_files {
        attachments.push(Attachment::Text {
            name: safe_basename(&file.name)?,
            text: checked_text(&file.text)?,
        });
    }
    validate_attachments(&attachments)?;
    Ok(attachments)
}

fn image_attachment(
    mime: Option<&str>,
    data: Option<&str>,
    name: Option<&str>,
) -> Result<Attachment, String> {
    let mime = mime
        .map(str::to_ascii_lowercase)
        .filter(|mime| {
            matches!(
                mime.as_str(),
                "image/png" | "image/jpeg" | "image/gif" | "image/webp"
            )
        })
        .ok_or("unsupported image MIME type")?;
    let data = data.ok_or("image attachment has no base64 data")?;
    if data.is_empty() || data.len() > MAX_IMAGE_BASE64 {
        return Err("image base64 data exceeds 10 MiB decoded limit or is empty".into());
    }
    let decoded = STANDARD
        .decode(data)
        .map_err(|_| "image data is not valid base64")?;
    if decoded.is_empty() || decoded.len() > MAX_IMAGE_BYTES {
        return Err("image exceeds 10 MiB decoded limit or is empty".into());
    }
    Ok(Attachment::Image {
        mime,
        data: data.to_owned(),
        name: name.map(safe_basename).transpose()?,
    })
}

fn checked_text(text: &str) -> Result<String, String> {
    if text.len() > MAX_TEXT_BYTES {
        return Err("text attachment exceeds 1 MiB".into());
    }
    Ok(text.to_owned())
}

fn validate_attachments(attachments: &[Attachment]) -> Result<(), String> {
    if attachments.len() > MAX_ATTACHMENTS {
        return Err("too many attachments (maximum 10)".into());
    }
    let mut total = 0usize;
    for attachment in attachments {
        total = total.saturating_add(match attachment {
            Attachment::Image { data, .. } => STANDARD
                .decode(data)
                .map_err(|_| "image data is not valid base64")?
                .len(),
            Attachment::Text { text, .. } => text.len(),
            Attachment::File { .. } => 0,
        });
        if total > MAX_TOTAL_BYTES {
            return Err("attachments exceed 20 MiB total".into());
        }
    }
    Ok(())
}

fn resource_name(uri: &str) -> Option<String> {
    let path = uri.split(['?', '#']).next()?;
    safe_basename(path).ok()
}

fn safe_basename(name: &str) -> Result<String, String> {
    let basename = name
        .trim()
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
        .split(['?', '#'])
        .next()
        .unwrap_or_default();
    if basename.is_empty() || basename.len() > 255 || basename.chars().any(char::is_control) {
        return Err("attachment filename is missing or malformed".into());
    }
    let safe: String = basename
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
                ch
            } else {
                '_'
            }
        })
        .collect();
    if safe.is_empty() || safe == "." || safe == ".." {
        return Err("attachment filename is missing or malformed".into());
    }
    Ok(safe)
}
