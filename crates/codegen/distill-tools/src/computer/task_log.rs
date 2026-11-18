//! Reads a bounded tail of a task's log file without loading all of it.

use std::path::Path;

use tokio::io::{AsyncReadExt, AsyncSeekExt};

/// Far more than any tool shows the model, so this bounds memory only. The
/// assertion pins the built-in budget; a runtime budget raised past this
/// would see only the tail of the log.
pub(crate) const MAX_SNAPSHOT_BYTES: usize = 1024 * 1024;
const _: () = assert!(MAX_SNAPSHOT_BYTES > crate::DEFAULT_TOOL_OUTPUT_BYTES);

/// Reads up to `max_bytes` from the end of `path`. An unreadable file returns
/// a bounded diagnostic and marks the output incomplete.
pub(crate) async fn read_tail(path: &Path, max_bytes: usize) -> (String, bool) {
    match read_bounded_tail(path, max_bytes).await {
        Ok((bytes, more)) => {
            let (text, cut) = decode_tail(&bytes, max_bytes);
            (text, more || cut)
        }
        Err(error) => {
            tracing::debug!(%error, path = %path.display(), "task log could not be read");
            (bounded_error(&error, max_bytes), true)
        }
    }
}

async fn read_bounded_tail(path: &Path, max_bytes: usize) -> std::io::Result<(Vec<u8>, bool)> {
    let mut file = tokio::fs::File::open(path).await?;
    let file_len = file.metadata().await?.len();
    let start = file_len.saturating_sub(max_bytes as u64);
    file.seek(std::io::SeekFrom::Start(start)).await?;

    let read_len = file_len - start;
    let mut buf = Vec::with_capacity(read_len as usize);
    file.take(read_len).read_to_end(&mut buf).await?;
    let more = start > 0 || (buf.len() as u64) < read_len;
    Ok((buf, more))
}

fn decode_tail(buf: &[u8], max_bytes: usize) -> (String, bool) {
    let (mut text, cut) = decode(buf);
    let mut truncated = cut;
    if text.len() > max_bytes {
        let mut start = text.len() - max_bytes;
        while !text.is_char_boundary(start) {
            start += 1;
        }
        text = text[start..].to_owned();
        truncated = true;
    }
    (text, truncated)
}

fn bounded_error(error: &std::io::Error, max_bytes: usize) -> String {
    let message = format!("[task output unavailable: {error}]");
    let mut end = max_bytes.min(message.len());
    while !message.is_char_boundary(end) {
        end -= 1;
    }
    message[..end].to_owned()
}

/// The flag reports a trailing split character that was dropped, so a cut
/// cannot read as the whole log.
fn decode(buf: &[u8]) -> (String, bool) {
    match std::str::from_utf8(buf) {
        Ok(text) => (text.to_owned(), false),
        Err(error) if error.error_len().is_none() => (
            String::from_utf8_lossy(buf.get(..error.valid_up_to()).unwrap_or(buf)).into_owned(),
            true,
        ),
        Err(_) => (String::from_utf8_lossy(buf).into_owned(), false),
    }
}

#[cfg(test)]
#[path = "task_log_tests.rs"]
mod tests;
