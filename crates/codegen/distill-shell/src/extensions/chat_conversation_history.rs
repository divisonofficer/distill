//! Disk-backed, cursor-paged transcript projection for Postudio.
//!
//! The first request for a legacy session scans and rewind-filters `updates.jsonl` to build
//! a durable projection. Later reads seek requested records and ingest only appended updates.
//! Rewind and truncation rebuild the projection and invalidate prior cursors.
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use agent_client_protocol as acp;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::ExtResult;
use crate::agent::MvpAgent;
use crate::session::info::Info;
use crate::session::storage as st;

const PAGE_SIZE: usize = 50;
const MAX_INDEX_BYTES: usize = 32 * 1024 * 1024;
const MAX_INDEX_ITEMS: usize = 20_000;
const MAX_MIGRATION_LINES: usize = 1_000_000;
const MAX_SOURCE_BYTES: u64 = 256 * 1024 * 1024;
const INDEX_VERSION: u32 = 3;
const INDEX_META: &str = "postudio-history.meta.json";
const INDEX_DATA: &str = "postudio-history.data.jsonl";
const INDEX_OFFSETS: &str = "postudio-history.offsets";
const INDEX_LOCK: &str = "postudio-history.lock";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Request {
    session_id: String,
    cwd: String,
    #[serde(default)]
    cursor: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
struct SourceStamp {
    len: u64,
    modified_secs: u64,
    modified_nanos: u32,
}

#[derive(Debug, Deserialize, Serialize)]
struct IndexMeta {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    generation: String,
    source: SourceStamp,
    #[serde(default)]
    source_offset: u64,
    count: u64,
    data_len: u64,
    #[serde(default)]
    live_len: u64,
    #[serde(default)]
    tools: std::collections::HashMap<String, u64>,
    #[serde(default)]
    last_delta: Option<LastDelta>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    usage: Option<Value>,
    #[serde(default)]
    orphaned_tasks: Vec<OrphanedTask>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct LastDelta {
    kind: String,
    index: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct OrphanedTask {
    task_id: String,
    command: String,
    cwd: String,
}

struct Page {
    items: Vec<Value>,
    #[cfg(test)]
    read_bytes: u64,
    #[cfg(test)]
    source_read_bytes: u64,
    #[cfg(test)]
    migrated: bool,
    cursor: Option<String>,
    has_more: bool,
    initial: bool,
    title: Option<String>,
    usage: Option<Value>,
}

#[tracing::instrument(skip_all, fields(method = %args.method))]
pub(crate) async fn handle(_agent: &MvpAgent, args: &acp::ExtRequest) -> ExtResult {
    let request: Request = super::parse_params(args)?;
    uuid::Uuid::try_parse(&request.session_id)
        .map_err(|_| acp::Error::invalid_params().data("sessionId must be a UUID"))?;
    let dir = resolve_session_dir(&request.session_id, &request.cwd)
        .ok_or_else(|| acp::Error::invalid_params().data("session not found"))?;
    let session_id = request.session_id;
    let cursor = request.cursor;
    let page = tokio::task::spawn_blocking(move || load_page(&dir, &session_id, cursor.as_deref()))
        .await
        .map_err(|error| acp::Error::internal_error().data(error.to_string()))?
        .map_err(|error| {
            if error.kind() == io::ErrorKind::InvalidInput {
                acp::Error::invalid_params().data(error.to_string())
            } else {
                acp::Error::internal_error().data(error.to_string())
            }
        })?;
    super::to_raw_response(&json!({
        "items": page.items,
        "cursor": page.cursor,
        "has_more": page.has_more,
        "initial": page.initial,
        "title": page.title,
        "usage": page.usage,
    }))
}

fn resolve_session_dir(session_id: &str, cwd: &str) -> Option<PathBuf> {
    let info = Info {
        id: acp::SessionId::new(session_id.to_owned()),
        cwd: cwd.to_owned(),
    };
    let direct = crate::session::persistence::session_dir(&info);
    if direct.join(st::SUMMARY_FILE).is_file() {
        return Some(direct);
    }
    crate::session::persistence::find_session_dir_by_id(session_id)
        .filter(|path| path.join(st::SUMMARY_FILE).is_file())
}

fn source_stamp(path: &Path) -> io::Result<SourceStamp> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(SourceStamp::default()),
        Err(error) => return Err(error),
    };
    if !metadata.file_type().is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "updates.jsonl is not a regular file",
        ));
    }
    let modified = metadata
        .modified()?
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?;
    Ok(SourceStamp {
        len: metadata.len(),
        modified_secs: modified.as_secs(),
        modified_nanos: modified.subsec_nanos(),
    })
}

fn index_paths(dir: &Path) -> (PathBuf, PathBuf, PathBuf, PathBuf) {
    (
        dir.join(INDEX_META),
        dir.join(INDEX_DATA),
        dir.join(INDEX_OFFSETS),
        dir.join(INDEX_LOCK),
    )
}

fn load_page(dir: &Path, session_id: &str, cursor: Option<&str>) -> io::Result<Page> {
    let updates = dir.join(st::UPDATES_FILE);
    let (meta_path, data_path, offsets_path, lock_path) = index_paths(dir);
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(lock_path)?;
    lock.lock_exclusive()?;
    let stamp = source_stamp(&updates)?;
    let cached = read_meta(&meta_path).ok().filter(|meta| {
        meta.version == INDEX_VERSION
            && !meta.generation.is_empty()
            && meta.count <= MAX_INDEX_ITEMS as u64
            && meta.orphaned_tasks.len() <= MAX_INDEX_ITEMS
            && meta.tools.len() <= MAX_INDEX_ITEMS
            && meta.source_offset <= meta.source.len
            && meta.data_len <= MAX_INDEX_BYTES as u64
            && meta.live_len <= meta.data_len
            && meta.live_len <= MAX_INDEX_BYTES as u64
            && regular_file_len(&data_path) == Some(meta.data_len)
            && regular_file_len(&offsets_path) == Some(meta.count.saturating_mul(16))
    });

    let mut _source_read_bytes = 0;
    let (meta, migrated) = match cached {
        None => {
            let (meta, bytes_read) =
                build_index(&updates, stamp, &meta_path, &data_path, &offsets_path)?;
            _source_read_bytes = bytes_read;
            (meta, true)
        }
        Some(meta) if meta.source == stamp => (meta, false),
        Some(meta) if stamp.len > meta.source.len => {
            let appended = read_appended_lines(&updates, meta.source_offset)?;
            _source_read_bytes = appended.bytes_read;
            if appended.lines.iter().any(|line| is_rewind_line(line)) {
                let (rebuilt, bytes_read) =
                    build_index(&updates, stamp, &meta_path, &data_path, &offsets_path)?;
                _source_read_bytes = bytes_read;
                (rebuilt, true)
            } else {
                let mut projection = Projection::from_meta(&meta);
                st::write_bytes_atomic(&meta_path, b"{}")?;
                let mut store = DiskStore::open(
                    &data_path,
                    &offsets_path,
                    meta.count,
                    meta.data_len,
                    meta.live_len,
                )?;
                for line in &appended.lines {
                    projection.consume(line, &mut store)?;
                    if projection.orphaned_tasks.len() > MAX_INDEX_ITEMS
                        || projection.tools.len() > MAX_INDEX_ITEMS
                    {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "Session history exceeds 20,000 items or background tasks",
                        ));
                    }
                }
                let updated = IndexMeta::from_projection(
                    stamp,
                    appended.offset,
                    store.count,
                    store.data_len,
                    store.live_len,
                    &projection,
                    meta.generation.clone(),
                );
                store.data.sync_all()?;
                store.offsets.sync_all()?;
                write_meta(&meta_path, &updated)?;
                (updated, false)
            }
        }
        Some(_) => {
            let (rebuilt, bytes_read) =
                build_index(&updates, stamp, &meta_path, &data_path, &offsets_path)?;
            _source_read_bytes = bytes_read;
            (rebuilt, true)
        }
    };

    let (start, end, initial, snapshot_count) = match cursor {
        None => (
            meta.count.saturating_sub(PAGE_SIZE as u64),
            meta.count,
            true,
            meta.count,
        ),
        Some(value) => {
            let parsed = decode_cursor(value, session_id)?;
            let stale = || {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "History cursor is stale; request latest history to recover",
                )
            };
            if parsed.generation != meta.generation || parsed.snapshot_count > meta.count {
                return Err(stale());
            }
            let first_before = parsed.snapshot_count.saturating_sub(PAGE_SIZE as u64);
            if parsed.before == 0
                || parsed.before > first_before
                || first_before.saturating_sub(parsed.before) % PAGE_SIZE as u64 != 0
            {
                return Err(stale());
            }
            (
                parsed.before.saturating_sub(PAGE_SIZE as u64),
                parsed.before,
                false,
                parsed.snapshot_count,
            )
        }
    };
    let has_more = start > 0;
    let next_cursor =
        has_more.then(|| encode_cursor(session_id, &meta.generation, snapshot_count, start));
    let (items, read_bytes) = read_records(&data_path, &offsets_path, start, end)?;
    if source_stamp(&updates)? != stamp {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Session history changed while paging",
        ));
    }
    let _ = FileExt::unlock(&lock);
    Ok(Page {
        items,
        #[cfg(test)]
        read_bytes,
        #[cfg(test)]
        source_read_bytes: _source_read_bytes,
        #[cfg(test)]
        migrated,
        cursor: next_cursor,
        has_more,
        initial,
        title: meta.title,
        usage: meta.usage,
    })
}

pub(crate) fn orphaned_background_tasks(
    updates_path: &Path,
) -> io::Result<Vec<(String, String, String)>> {
    if updates_path.file_name() != Some(std::ffi::OsStr::new(st::UPDATES_FILE)) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid updates path",
        ));
    }
    let dir = updates_path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "updates path has no parent"))?;
    let _ = load_page(dir, "", None)?;
    let (meta_path, _, _, _) = index_paths(dir);
    let meta = read_meta(&meta_path)?;
    if source_stamp(updates_path)? != meta.source {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "session history changed while indexing",
        ));
    }
    Ok(meta
        .orphaned_tasks
        .into_iter()
        .map(|task| (task.task_id, task.command, task.cwd))
        .collect())
}

struct HistoryCursor {
    generation: String,
    snapshot_count: u64,
    before: u64,
}

fn encode_cursor(session_id: &str, generation: &str, snapshot_count: u64, before: u64) -> String {
    format!("2:{session_id}:{generation}:{snapshot_count}:{before}")
}

fn decode_cursor(cursor: &str, session_id: &str) -> io::Result<HistoryCursor> {
    let fields: Vec<_> = cursor.split(':').collect();
    if fields.len() != 5 || fields[0] != "2" || fields[1] != session_id {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "History cursor is invalid or belongs to another session",
        ));
    }
    let snapshot_count = fields[3]
        .parse::<u64>()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "History cursor is malformed"))?;
    let before = fields[4]
        .parse::<u64>()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "History cursor is malformed"))?;
    Ok(HistoryCursor {
        generation: fields[2].to_owned(),
        snapshot_count,
        before,
    })
}

fn regular_file_len(path: &Path) -> Option<u64> {
    std::fs::symlink_metadata(path)
        .ok()
        .filter(|metadata| metadata.file_type().is_file())
        .map(|metadata| metadata.len())
}

fn read_meta(path: &Path) -> io::Result<IndexMeta> {
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() || metadata.len() > MAX_INDEX_BYTES as u64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "history index metadata is not a regular file within 32 MiB",
        ));
    }
    let bytes = std::fs::read(path)?;
    serde_json::from_slice(&bytes).map_err(io::Error::other)
}

struct AppendedLines {
    lines: Vec<String>,
    offset: u64,
    bytes_read: u64,
}

fn read_appended_lines(path: &Path, offset: u64) -> io::Result<AppendedLines> {
    let mut reader = BufReader::new(File::open(path)?);
    reader.seek(SeekFrom::Start(offset))?;
    let mut lines = Vec::new();
    let mut position = offset;
    let mut bytes_read = 0u64;
    loop {
        let mut raw = Vec::new();
        let len = reader.read_until(b'\n', &mut raw)?;
        if len == 0 {
            break;
        }
        bytes_read = bytes_read.saturating_add(len as u64);
        if bytes_read > MAX_INDEX_BYTES as u64 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Appended session history exceeds 32 MiB",
            ));
        }
        if raw.last() != Some(&b'\n') {
            break;
        }
        position = position.saturating_add(len as u64);
        raw.pop();
        if raw.last() == Some(&b'\r') {
            raw.pop();
        }
        if !raw.iter().all(u8::is_ascii_whitespace) {
            lines.push(String::from_utf8(raw).map_err(io::Error::other)?);
        }
    }
    Ok(AppendedLines {
        lines,
        offset: position,
        bytes_read,
    })
}

fn is_rewind_line(line: &str) -> bool {
    serde_json::from_str::<Value>(line)
        .ok()
        .is_some_and(|value| {
            value.get("method").and_then(Value::as_str) == Some(st::XAI_SESSION_UPDATE_METHOD)
                && value
                    .pointer("/params/update/sessionUpdate")
                    .and_then(Value::as_str)
                    == Some("rewind_marker")
        })
}

trait ProjectionStore {
    fn count(&self) -> u64;
    fn data_len(&self) -> u64;
    fn live_len(&self) -> u64;
    fn record_len(&mut self, index: u64) -> io::Result<u64>;
    fn reclaimable(&mut self, index: u64) -> io::Result<u64>;
    fn get(&mut self, index: u64) -> io::Result<Value>;
    fn append(&mut self, bytes: &[u8]) -> io::Result<u64>;
    fn replace(&mut self, index: u64, bytes: &[u8]) -> io::Result<()>;
}

#[derive(Default)]
struct MemoryStore {
    data: Vec<u8>,
    offsets: Vec<(u64, u64)>,
    live_len: u64,
}

impl MemoryStore {
    fn maybe_compact(&mut self) {
        let obsolete = self.data.len() as u64 - self.live_len;
        let obsolete_limit = self.live_len.saturating_mul(16).max(64 * 1024);
        if self.data.len() as u64 <= MAX_INDEX_BYTES as u64 && obsolete <= obsolete_limit {
            return;
        }
        let mut compacted = Vec::with_capacity(self.live_len as usize);
        for (offset, len) in &mut self.offsets {
            let start = *offset as usize;
            let end = start + *len as usize;
            *offset = compacted.len() as u64;
            compacted.extend_from_slice(&self.data[start..end]);
        }
        self.data = compacted;
    }

    fn encode_offsets(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.offsets.len() * 16);
        for (offset, len) in &self.offsets {
            bytes.extend_from_slice(&offset.to_le_bytes());
            bytes.extend_from_slice(&len.to_le_bytes());
        }
        bytes
    }
}

impl ProjectionStore for MemoryStore {
    fn count(&self) -> u64 {
        self.offsets.len() as u64
    }

    fn data_len(&self) -> u64 {
        self.data.len() as u64
    }

    fn live_len(&self) -> u64 {
        self.live_len
    }

    fn record_len(&mut self, index: u64) -> io::Result<u64> {
        self.offsets
            .get(index as usize)
            .map(|(_, len)| *len)
            .ok_or_else(|| io::Error::other("invalid history index"))
    }

    fn reclaimable(&mut self, index: u64) -> io::Result<u64> {
        let (offset, len) = self
            .offsets
            .get(index as usize)
            .copied()
            .ok_or_else(|| io::Error::other("invalid history index"))?;
        Ok(if offset + len == self.data_len() {
            len
        } else {
            0
        })
    }

    fn get(&mut self, index: u64) -> io::Result<Value> {
        let (offset, len) = *self
            .offsets
            .get(index as usize)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid history index"))?;
        let end = offset
            .checked_add(len)
            .filter(|end| *end <= self.data.len() as u64)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "history index offsets are corrupt",
                )
            })?;
        serde_json::from_slice(&self.data[offset as usize..end as usize]).map_err(io::Error::other)
    }

    fn append(&mut self, bytes: &[u8]) -> io::Result<u64> {
        let index = self.count();
        let offset = self.data.len() as u64;
        self.data.extend_from_slice(bytes);
        self.data.push(b'\n');
        let len = bytes.len() as u64 + 1;
        self.offsets.push((offset, len));
        self.live_len += len;
        self.maybe_compact();
        Ok(index)
    }

    fn replace(&mut self, index: u64, bytes: &[u8]) -> io::Result<()> {
        let (old_offset, old_len) =
            self.offsets.get(index as usize).copied().ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "invalid history index")
            })?;
        if old_offset + old_len == self.data.len() as u64 {
            self.data.truncate(old_offset as usize);
        }
        let offset = self.data.len() as u64;
        self.data.extend_from_slice(bytes);
        self.data.push(b'\n');
        self.offsets[index as usize] = (offset, bytes.len() as u64 + 1);
        self.live_len = self.live_len - old_len + bytes.len() as u64 + 1;
        self.maybe_compact();
        Ok(())
    }
}

struct DiskStore {
    data: File,
    offsets: File,
    count: u64,
    data_len: u64,
    live_len: u64,
}

impl DiskStore {
    fn open(
        data_path: &Path,
        offsets_path: &Path,
        count: u64,
        data_len: u64,
        live_len: u64,
    ) -> io::Result<Self> {
        Ok(Self {
            data: OpenOptions::new().read(true).write(true).open(data_path)?,
            offsets: OpenOptions::new()
                .read(true)
                .write(true)
                .open(offsets_path)?,
            count,
            data_len,
            live_len,
        })
    }

    fn compact(&mut self) -> io::Result<()> {
        let mut records = Vec::with_capacity(self.count as usize);
        for index in 0..self.count {
            self.offsets.seek(SeekFrom::Start(index * 16))?;
            let mut raw = [0; 16];
            self.offsets.read_exact(&mut raw)?;
            records.push((
                u64::from_le_bytes(raw[..8].try_into().unwrap()),
                u64::from_le_bytes(raw[8..].try_into().unwrap()),
            ));
        }
        let mut compacted = Vec::with_capacity(self.live_len as usize);
        for (index, (offset, len)) in records.into_iter().enumerate() {
            if offset
                .checked_add(len)
                .is_none_or(|end| end > self.data_len)
            {
                return Err(io::Error::other("history index offsets are corrupt"));
            }
            if len > MAX_INDEX_BYTES as u64 || compacted.len() as u64 + len > MAX_INDEX_BYTES as u64
            {
                return Err(io::Error::other("history index live data exceeds 32 MiB"));
            }
            let start = compacted.len() as u64;
            let mut bytes = vec![0; len as usize];
            self.data.seek(SeekFrom::Start(offset))?;
            self.data.read_exact(&mut bytes)?;
            compacted.extend_from_slice(&bytes);
            self.offsets.seek(SeekFrom::Start(index as u64 * 16))?;
            self.offsets.write_all(&start.to_le_bytes())?;
            self.offsets.write_all(&len.to_le_bytes())?;
        }
        if compacted.len() as u64 != self.live_len {
            return Err(io::Error::other("history index live byte count is corrupt"));
        }
        self.data.seek(SeekFrom::Start(0))?;
        self.data.set_len(0)?;
        self.data.write_all(&compacted)?;
        self.data_len = compacted.len() as u64;
        Ok(())
    }

    fn maybe_compact(&mut self) -> io::Result<()> {
        let obsolete = self.data_len - self.live_len;
        let obsolete_limit = self.live_len.saturating_mul(16).max(64 * 1024);
        if self.data_len <= MAX_INDEX_BYTES as u64 && obsolete <= obsolete_limit {
            return Ok(());
        }
        self.compact()
    }

    fn append_record(&mut self, bytes: &[u8]) -> io::Result<(u64, u64)> {
        let offset = self.data_len;
        let len = bytes.len() as u64 + 1;
        self.data.seek(SeekFrom::Start(offset))?;
        self.data.write_all(bytes)?;
        self.data.write_all(b"\n")?;
        self.data_len = self.data_len.saturating_add(len);
        Ok((offset, len))
    }

    fn write_offset(&mut self, index: u64, offset: u64, len: u64) -> io::Result<()> {
        self.offsets
            .seek(SeekFrom::Start(index.saturating_mul(16)))?;
        self.offsets.write_all(&offset.to_le_bytes())?;
        self.offsets.write_all(&len.to_le_bytes())
    }
}

impl ProjectionStore for DiskStore {
    fn count(&self) -> u64 {
        self.count
    }

    fn data_len(&self) -> u64 {
        self.data_len
    }

    fn live_len(&self) -> u64 {
        self.live_len
    }

    fn record_len(&mut self, index: u64) -> io::Result<u64> {
        if index >= self.count {
            return Err(io::Error::other("invalid history index"));
        }
        self.offsets.seek(SeekFrom::Start(index * 16))?;
        let mut raw = [0; 16];
        self.offsets.read_exact(&mut raw)?;
        Ok(u64::from_le_bytes(raw[8..].try_into().unwrap()))
    }

    fn reclaimable(&mut self, index: u64) -> io::Result<u64> {
        if index >= self.count {
            return Err(io::Error::other("invalid history index"));
        }
        self.offsets.seek(SeekFrom::Start(index * 16))?;
        let mut raw = [0; 16];
        self.offsets.read_exact(&mut raw)?;
        let offset = u64::from_le_bytes(raw[..8].try_into().unwrap());
        let len = u64::from_le_bytes(raw[8..].try_into().unwrap());
        Ok(if offset.checked_add(len) == Some(self.data_len) {
            len
        } else {
            0
        })
    }

    fn get(&mut self, index: u64) -> io::Result<Value> {
        if index >= self.count {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid history index",
            ));
        }
        self.offsets
            .seek(SeekFrom::Start(index.saturating_mul(16)))?;
        let mut raw = [0; 16];
        self.offsets.read_exact(&mut raw)?;
        let offset = u64::from_le_bytes(raw[..8].try_into().unwrap());
        let len = u64::from_le_bytes(raw[8..].try_into().unwrap());
        let end = offset
            .checked_add(len)
            .filter(|end| *end <= self.data_len)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "history index offsets are corrupt",
                )
            })?;
        if len > MAX_INDEX_BYTES as u64 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "history index record exceeds 32 MiB",
            ));
        }
        let mut bytes = vec![0; len as usize];
        self.data.seek(SeekFrom::Start(offset))?;
        self.data.read_exact(&mut bytes)?;
        let _ = end;
        serde_json::from_slice(&bytes).map_err(io::Error::other)
    }

    fn append(&mut self, bytes: &[u8]) -> io::Result<u64> {
        let index = self.count;
        let (offset, len) = self.append_record(bytes)?;
        self.write_offset(index, offset, len)?;
        self.count += 1;
        self.live_len += len;
        self.maybe_compact()?;
        Ok(index)
    }

    fn replace(&mut self, index: u64, bytes: &[u8]) -> io::Result<()> {
        if index >= self.count {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid history index",
            ));
        }
        let old_len = self.record_len(index)?;
        let reclaimed = self.reclaimable(index)?;
        if reclaimed > 0 {
            self.data_len -= reclaimed;
            self.data.set_len(self.data_len)?;
        }
        let (offset, len) = self.append_record(bytes)?;
        self.write_offset(index, offset, len)?;
        self.live_len = self.live_len - old_len + len;
        self.maybe_compact()
    }
}

fn write_meta(path: &Path, meta: &IndexMeta) -> io::Result<()> {
    let bytes = serde_json::to_vec(meta).map_err(io::Error::other)?;
    if bytes.len() > MAX_INDEX_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Session history metadata exceeds 32 MiB",
        ));
    }
    st::write_bytes_atomic(path, &bytes)
}

fn build_index(
    updates: &Path,
    source: SourceStamp,
    meta_path: &Path,
    data_path: &Path,
    offsets_path: &Path,
) -> io::Result<(IndexMeta, u64)> {
    // Rewind can remove arbitrary prior turns, so legacy migration scans a bounded source once.
    if source.len > MAX_SOURCE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Legacy session history exceeds 256 MiB",
        ));
    }
    let mut lines = Vec::new();
    let mut source_read_bytes = 0u64;
    let mut source_offset = 0u64;
    if updates.exists() {
        let mut reader = BufReader::new(File::open(updates)?);
        loop {
            let mut raw = Vec::new();
            let len = reader.read_until(b'\n', &mut raw)?;
            if len == 0 {
                break;
            }
            source_read_bytes = source_read_bytes.saturating_add(len as u64);
            if source_read_bytes > MAX_SOURCE_BYTES {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Legacy session history exceeds 256 MiB",
                ));
            }
            if raw.last() != Some(&b'\n') {
                break;
            }
            source_offset = source_offset.saturating_add(len as u64);
            raw.pop();
            if raw.last() == Some(&b'\r') {
                raw.pop();
            }
            if !raw.iter().all(u8::is_ascii_whitespace) {
                if lines.len() >= MAX_MIGRATION_LINES {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Legacy session history exceeds 1,000,000 lines",
                    ));
                }
                lines.push(String::from_utf8(raw).map_err(io::Error::other)?);
            }
        }
    }
    let filtered = st::filter_rewind_lines(lines.iter().map(String::as_str).collect());
    let mut projection = Projection::default();
    let mut store = MemoryStore::default();
    for line in filtered {
        projection.consume(line, &mut store)?;
        if store.live_len() > MAX_INDEX_BYTES as u64 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Session history exceeds 32 MiB",
            ));
        }
        if store.count() > MAX_INDEX_ITEMS as u64
            || projection.orphaned_tasks.len() > MAX_INDEX_ITEMS
            || projection.tools.len() > MAX_INDEX_ITEMS
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Session history exceeds 20,000 items or background tasks",
            ));
        }
    }
    let meta = IndexMeta::from_projection(
        source,
        source_offset,
        store.count(),
        store.data_len(),
        store.live_len(),
        &projection,
        uuid::Uuid::new_v4().to_string(),
    );
    st::write_bytes_atomic(meta_path, b"{}")?;
    st::write_bytes_atomic(data_path, &store.data)?;
    st::write_bytes_atomic(offsets_path, &store.encode_offsets())?;
    write_meta(meta_path, &meta)?;
    Ok((meta, source_read_bytes))
}

fn read_records(
    data_path: &Path,
    offsets_path: &Path,
    start: u64,
    end: u64,
) -> io::Result<(Vec<Value>, u64)> {
    let count = end.saturating_sub(start);
    let mut offsets_file = File::open(offsets_path)?;
    offsets_file.seek(SeekFrom::Start(start.saturating_mul(16)))?;
    let mut offsets = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let mut raw = [0; 16];
        offsets_file.read_exact(&mut raw)?;
        offsets.push((
            u64::from_le_bytes(raw[..8].try_into().unwrap()),
            u64::from_le_bytes(raw[8..].try_into().unwrap()),
        ));
    }
    let mut data_file = File::open(data_path)?;
    let data_len = data_file.metadata()?.len();
    if offsets.iter().any(|(offset, len)| {
        offset
            .checked_add(*len)
            .is_none_or(|end| end > data_len || *len > MAX_INDEX_BYTES as u64)
    }) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "history index offsets are corrupt",
        ));
    }
    let mut items = Vec::with_capacity(count as usize);
    let mut bytes_read = count.saturating_mul(16);
    for (offset, len) in offsets {
        let mut bytes = vec![0; len as usize];
        data_file.seek(SeekFrom::Start(offset))?;
        data_file.read_exact(&mut bytes)?;
        bytes_read = bytes_read.saturating_add(len);
        items.push(serde_json::from_slice(&bytes).map_err(io::Error::other)?);
    }
    Ok((items, bytes_read))
}

#[derive(Default)]
struct Projection {
    tools: std::collections::HashMap<String, u64>,
    last_delta: Option<LastDelta>,
    title: Option<String>,
    usage: Option<Value>,
    orphaned_tasks: std::collections::HashMap<String, OrphanedTask>,
}

impl IndexMeta {
    fn from_projection(
        source: SourceStamp,
        source_offset: u64,
        count: u64,
        data_len: u64,
        live_len: u64,
        projection: &Projection,
        generation: String,
    ) -> Self {
        Self {
            version: INDEX_VERSION,
            generation,
            source,
            source_offset,
            count,
            data_len,
            live_len,
            tools: projection.tools.clone(),
            last_delta: projection.last_delta.clone(),
            title: projection.title.clone(),
            usage: projection.usage.clone(),
            orphaned_tasks: projection.orphaned_tasks.values().cloned().collect(),
        }
    }
}

impl Projection {
    fn from_meta(meta: &IndexMeta) -> Self {
        Self {
            tools: meta.tools.clone(),
            last_delta: meta.last_delta.clone(),
            title: meta.title.clone(),
            usage: meta.usage.clone(),
            orphaned_tasks: meta
                .orphaned_tasks
                .iter()
                .cloned()
                .map(|task| (task.task_id.clone(), task))
                .collect(),
        }
    }

    fn write_record(
        &self,
        store: &mut impl ProjectionStore,
        index: Option<u64>,
        item: &Value,
    ) -> io::Result<u64> {
        let encoded = serde_json::to_vec(item).map_err(io::Error::other)?;
        let new_len = store
            .live_len()
            .saturating_sub(match index {
                Some(index) => store.record_len(index)?,
                None => 0,
            })
            .checked_add(encoded.len() as u64 + 1)
            .filter(|len| *len <= MAX_INDEX_BYTES as u64)
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "Session history exceeds 32 MiB")
            })?;
        match index {
            Some(index) => {
                store.replace(index, &encoded)?;
                Ok(index)
            }
            None => {
                if store.count() >= MAX_INDEX_ITEMS as u64 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Session history exceeds 20,000 items",
                    ));
                }
                let index = store.append(&encoded)?;
                debug_assert_eq!(store.live_len(), new_len);
                Ok(index)
            }
        }
    }

    fn push(&mut self, item: Value, store: &mut impl ProjectionStore) -> io::Result<()> {
        let kind = item.get("t").and_then(Value::as_str);
        if matches!(kind, Some("message.delta" | "reasoning.delta"))
            && let Some(last) = &self.last_delta
            && Some(last.kind.as_str()) == kind
        {
            let mut previous = store.get(last.index)?;
            let old = previous
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let new = item.get("text").and_then(Value::as_str).unwrap_or_default();
            previous["text"] = json!(format!("{old}{new}"));
            self.write_record(store, Some(last.index), &previous)?;
            return Ok(());
        }
        if kind == Some("tool")
            && let Some(id) = item.get("id").and_then(Value::as_str)
        {
            if let Some(index) = self.tools.get(id).copied() {
                let mut old = store.get(index)?;
                for field in ["name", "title", "kind", "status", "input", "content"] {
                    if let Some(value) = item.get(field).filter(|value| !value.is_null()) {
                        old[field] = value.clone();
                    }
                }
                self.write_record(store, Some(index), &old)?;
                return Ok(());
            }
            self.tools.insert(id.to_owned(), store.count());
        }
        let index = self.write_record(store, None, &item)?;
        self.last_delta =
            matches!(kind, Some("message.delta" | "reasoning.delta")).then(|| LastDelta {
                kind: kind.unwrap_or_default().to_owned(),
                index,
            });
        Ok(())
    }

    fn consume(&mut self, line: &str, store: &mut impl ProjectionStore) -> io::Result<()> {
        let Ok(envelope) = serde_json::from_str::<Value>(line) else {
            return Ok(());
        };
        let method = envelope
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let Some(params) = envelope.get("params") else {
            return Ok(());
        };
        let Some(update) = params.get("update") else {
            return Ok(());
        };
        let Some(tag) = update.get("sessionUpdate").and_then(Value::as_str) else {
            return Ok(());
        };
        if method == st::XAI_SESSION_UPDATE_METHOD {
            match tag {
                "task_backgrounded" => {
                    if let Some(task_id) = update.get("task_id").and_then(Value::as_str) {
                        self.orphaned_tasks.insert(
                            task_id.to_owned(),
                            OrphanedTask {
                                task_id: task_id.to_owned(),
                                command: update
                                    .get("command")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default()
                                    .to_owned(),
                                cwd: update
                                    .get("cwd")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default()
                                    .to_owned(),
                            },
                        );
                    }
                    return Ok(());
                }
                "task_completed" => {
                    if let Some(task_id) = update
                        .pointer("/task_snapshot/task_id")
                        .and_then(Value::as_str)
                    {
                        self.orphaned_tasks.remove(task_id);
                    }
                    return Ok(());
                }
                "turn_completed" => {}
                _ => return Ok(()),
            }
        }
        let content = update.get("content").unwrap_or(&Value::Null);
        if hidden(update, content) {
            return Ok(());
        }
        match tag {
            "user_message_chunk" => {
                if let Some(text) = text_content(content) {
                    self.push(json!({"t":"user.message", "text":text}), store)?;
                } else if let Some(attachment) = attachment(content) {
                    self.push(
                        json!({"t":"user.attachment", "attachment":attachment}),
                        store,
                    )?;
                }
            }
            "agent_message_chunk" => {
                if let Some(text) = text_content(content) {
                    self.push(json!({"t":"message.delta", "text":text}), store)?;
                }
            }
            "agent_thought_chunk" => {
                if let Some(text) = text_content(content) {
                    self.push(json!({"t":"reasoning.delta", "text":text}), store)?;
                }
            }
            "tool_call" => {
                let Some(id) = update.get("toolCallId").and_then(Value::as_str) else {
                    return Ok(());
                };
                let meta = update.pointer("/_meta/x.ai~1tool");
                let mut item = json!({"t":"tool", "id":id, "status":status(update).unwrap_or_else(|| "pending".to_owned())});
                for (field, value) in [
                    ("name", meta.and_then(|metadata| metadata.get("name"))),
                    ("kind", meta.and_then(|metadata| metadata.get("kind"))),
                    ("title", update.get("title")),
                    ("input", update.get("rawInput")),
                ] {
                    if let Some(value) = value {
                        item[field] = value.clone();
                    }
                }
                if let Some(contents) = tool_content(update) {
                    item["content"] = contents;
                }
                self.push(item, store)?;
            }
            "tool_call_update" => {
                let Some(id) = update.get("toolCallId").and_then(Value::as_str) else {
                    return Ok(());
                };
                let mut item = json!({"t":"tool", "id":id});
                if let Some(status) = status(update) {
                    item["status"] = json!(status);
                }
                if let Some(contents) = tool_content(update) {
                    item["content"] = contents;
                }
                if item.as_object().is_some_and(|map| map.len() > 2) {
                    self.push(item, store)?;
                }
            }
            "session_info_update" => {
                if let Some(title) = update.get("title").and_then(Value::as_str) {
                    self.title = Some(title.to_owned());
                }
            }
            "usage_update" => {
                if let (Some(used), Some(size)) = (
                    update.get("used").and_then(Value::as_u64),
                    update.get("size").and_then(Value::as_u64),
                ) {
                    self.usage = Some(json!({"used":used, "size":size}));
                }
            }
            "turn_completed" => {
                if let Some(stop_reason) = update.get("stop_reason").and_then(Value::as_str) {
                    let mut item = json!({"t":"turn.end", "stop_reason":stop_reason});
                    if stop_reason == "error"
                        && let Some(error) = update.get("agent_result").and_then(Value::as_str)
                    {
                        item["error"] = json!(error);
                    }
                    self.push(item, store)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}

fn text_content(content: &Value) -> Option<&str> {
    (content.get("type")?.as_str()? == "text")
        .then(|| content.get("text")?.as_str())
        .flatten()
}

fn hidden(update: &Value, content: &Value) -> bool {
    [update, content].iter().any(|value| {
        ["_meta", "meta"].iter().any(|key| {
            value
                .pointer(&format!("/{key}/hideFromScrollback"))
                .and_then(Value::as_bool)
                == Some(true)
        })
    })
}

fn status(update: &Value) -> Option<String> {
    update
        .get("status")?
        .as_str()
        .map(|value| match value.to_ascii_lowercase().as_str() {
            "inprogress" | "in_progress" => "in_progress".to_owned(),
            other => other.to_owned(),
        })
}

fn attachment(content: &Value) -> Option<Value> {
    match content.get("type")?.as_str()? {
        "image" => {
            use base64::Engine as _;
            let mime = content.get("mimeType")?.as_str()?.to_ascii_lowercase();
            if !matches!(
                mime.as_str(),
                "image/png" | "image/jpeg" | "image/gif" | "image/webp"
            ) {
                return None;
            }
            let data = content.get("data")?.as_str()?;
            if data.is_empty() || data.len() > 4 * 10 * 1024 * 1024 / 3 + 4 {
                return None;
            }
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(data)
                .ok()?;
            if decoded.is_empty() || decoded.len() > 10 * 1024 * 1024 {
                return None;
            }
            let name = match content.get("name").and_then(Value::as_str) {
                Some(name) => Some(safe_basename(name)?),
                None => None,
            };
            Some(json!({"kind":"image", "mime":mime, "data":data, "name":name}))
        }
        "resource" => {
            let resource = content.get("resource").unwrap_or(content);
            let text = resource.get("text")?.as_str()?;
            if text.len() > 1024 * 1024 {
                return None;
            }
            let uri = resource.get("uri")?.as_str()?;
            let name = safe_basename(uri.split(['?', '#']).next()?)?;
            Some(json!({"kind":"text", "name":name, "text":text}))
        }
        "resource_link" => {
            let name = safe_basename(content.get("name")?.as_str()?)?;
            Some(json!({"kind":"file", "name":name, "uri":content.get("uri")}))
        }
        _ => None,
    }
}

fn safe_basename(name: &str) -> Option<String> {
    let basename = name
        .trim()
        .rsplit(['/', '\\'])
        .next()?
        .split(['?', '#'])
        .next()?;
    if basename.is_empty() || basename.len() > 255 || basename.chars().any(char::is_control) {
        return None;
    }
    let safe: String = basename
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-') {
                character
            } else {
                '_'
            }
        })
        .collect();
    (!safe.is_empty() && safe != "." && safe != "..").then_some(safe)
}

fn tool_content(update: &Value) -> Option<Value> {
    let content = update.get("content")?.as_array()?;
    let items: Vec<Value> = content.iter().filter_map(|item| match item.get("type")?.as_str()? {
        "diff" => Some(json!({"type":"diff", "path":item.get("path")?, "old":item.get("oldText"), "new":item.get("newText")?})),
        "content" => {
            let inner = item.get("content")?;
            match inner.get("type")?.as_str()? {
                "text" => Some(json!({"type":"text", "text":inner.get("text")?})),
                "image" => Some(json!({"type":"image", "mime":inner.get("mimeType")?, "data":inner.get("data")?})),
                _ => None,
            }
        }
        _ => None,
    }).collect();
    Some(json!(items))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn write_updates(dir: &Path, count: usize) {
        let mut file = File::create(dir.join(st::UPDATES_FILE)).unwrap();
        for i in 0..count {
            writeln!(file, "{{\"method\":\"session/update\",\"params\":{{\"update\":{{\"sessionUpdate\":\"user_message_chunk\",\"content\":{{\"type\":\"text\",\"text\":\"m{i}\"}}}}}}}}").unwrap();
        }
    }

    #[test]
    fn index_pages_read_only_bounded_records_and_cursor_rejects_bad_stale_or_foreign() {
        let dir = tempfile::tempdir().unwrap();
        let sid = "00000000-0000-0000-0000-000000000001";
        write_updates(dir.path(), 500);
        let first = load_page(dir.path(), sid, None).unwrap();
        assert_eq!(first.items.len(), 50);
        assert!(first.migrated);
        assert_eq!(first.items.first().unwrap()["text"], "m450");
        let cursor = first.cursor.as_deref().unwrap();
        let second = load_page(dir.path(), sid, Some(cursor)).unwrap();
        assert_eq!(second.items.first().unwrap()["text"], "m400");
        assert_eq!(second.items.last().unwrap()["text"], "m449");
        assert!(!second.migrated);
        let indexed_bytes = read_meta(&dir.path().join(INDEX_META)).unwrap().data_len;
        assert!(
            second.read_bytes < indexed_bytes,
            "older page read entire index"
        );
        let repeated = load_page(dir.path(), sid, None).unwrap();
        assert_eq!(repeated.items.len(), 50);
        assert!(!repeated.migrated);
        assert!(
            repeated.read_bytes < indexed_bytes,
            "latest page read entire index"
        );
        assert!(
            load_page(
                dir.path(),
                "00000000-0000-0000-0000-000000000002",
                Some(cursor)
            )
            .is_err()
        );
        assert!(load_page(dir.path(), sid, Some("bad:cursor")).is_err());
        let forged = encode_cursor(
            sid,
            &read_meta(&dir.path().join(INDEX_META)).unwrap().generation,
            500,
            449,
        );
        assert!(
            load_page(dir.path(), sid, Some(&forged)).is_err(),
            "non-page boundary accepted"
        );
        let mut f = OpenOptions::new()
            .append(true)
            .open(dir.path().join(st::UPDATES_FILE))
            .unwrap();
        f.write_all(br#"{"method":"session/update","params":{"update":{"sessionUpdate":"user_message_chunk","content":{"type":"text","text":"new"}}}}"#).unwrap();
        f.write_all(b"\n").unwrap();
        let appended = load_page(dir.path(), sid, Some(cursor)).unwrap();
        assert!(!appended.migrated);
        assert!(appended.source_read_bytes < 512);
        assert_eq!(appended.items.first().unwrap()["text"], "m400");
        assert_eq!(appended.items.last().unwrap()["text"], "m449");
        let refreshed = load_page(dir.path(), sid, None).unwrap();
        assert_eq!(refreshed.items.last().unwrap()["text"], "new");
    }

    #[test]
    fn projection_preserves_rewind_and_collapsed_tool_updates() {
        let rewind = r#"{"method":"_x.ai/session/update","params":{"update":{"sessionUpdate":"rewind_marker","target_prompt_index":1}}}"#;
        let lines = vec![
            r#"{"method":"session/update","params":{"update":{"_meta":{"promptIndex":0},"sessionUpdate":"user_message_chunk","content":{"type":"text","text":"old"}}}}"#.to_owned(),
            r#"{"method":"session/update","params":{"update":{"_meta":{"promptIndex":1},"sessionUpdate":"user_message_chunk","content":{"type":"text","text":"dead"}}}}"#.to_owned(),
            rewind.to_owned(),
            r#"{"method":"session/update","params":{"update":{"sessionUpdate":"tool_call","toolCallId":"t1","title":"cmd","status":"in_progress","rawInput":{"x":1}}}}"#.to_owned(),
            r#"{"method":"session/update","params":{"update":{"sessionUpdate":"tool_call_update","toolCallId":"t1","status":"completed"}}}"#.to_owned(),
        ];
        let filtered = st::filter_rewind_lines(lines.iter().map(String::as_str).collect());
        let mut projected = Projection::default();
        let mut store = MemoryStore::default();
        for line in filtered {
            projected.consume(line, &mut store).unwrap();
        }
        assert_eq!(store.count(), 2);
        let user = store.get(0).unwrap();
        let tool = store.get(1).unwrap();
        assert_eq!(user["text"], "old");
        assert_eq!(tool["status"], "completed");

        let mut task_projection = Projection::default();
        task_projection.consume(r#"{"method":"_x.ai/session/update","params":{"update":{"sessionUpdate":"task_backgrounded","task_id":"done","command":"true","cwd":"/tmp"}}}"#, &mut store).unwrap();
        task_projection.consume(r#"{"method":"_x.ai/session/update","params":{"update":{"sessionUpdate":"task_completed","task_snapshot":{"task_id":"done"}}}}"#, &mut store).unwrap();
        task_projection.consume(r#"{"method":"_x.ai/session/update","params":{"update":{"sessionUpdate":"task_backgrounded","task_id":"orphan","command":"sleep","cwd":"/tmp"}}}"#, &mut store).unwrap();
        assert_eq!(task_projection.orphaned_tasks.len(), 1);
        assert!(task_projection.orphaned_tasks.contains_key("orphan"));
    }
    #[test]
    fn persisted_turn_endings_preserve_boundaries_and_errors() {
        let mut projection = Projection::default();
        let mut store = MemoryStore::default();
        for reason in ["end_turn", "error", "cancelled"] {
            projection.consume(&json!({"method":"session/update","params":{"update":{
                "sessionUpdate":"agent_message_chunk", "content":{"type":"text","text":"answer"}
            }}}).to_string(), &mut store).unwrap();
            projection.consume(&json!({"method":"_x.ai/session/update","params":{"update":{
                "sessionUpdate":"turn_completed", "stop_reason":reason, "agent_result":"failed"
            }}}).to_string(), &mut store).unwrap();
        }
        assert_eq!(store.count(), 6);
        assert_eq!(
            store.get(1).unwrap(),
            json!({"t":"turn.end","stop_reason":"end_turn"})
        );
        assert_eq!(
            store.get(3).unwrap(),
            json!({"t":"turn.end","stop_reason":"error","error":"failed"})
        );
        assert_eq!(store.get(5).unwrap()["stop_reason"], "cancelled");
    }

    #[test]
    fn appended_rewind_invalidates_cursor_and_latest_recovers() {
        let dir = tempfile::tempdir().unwrap();
        write_updates(dir.path(), 100);
        let first = load_page(dir.path(), "s", None).unwrap();
        let mut file = OpenOptions::new()
            .append(true)
            .open(dir.path().join(st::UPDATES_FILE))
            .unwrap();
        writeln!(
            file,
            "{}",
            json!({"method":"_x.ai/session/update","params":{"update":{
                "sessionUpdate":"rewind_marker","target_prompt_index":0
            }}})
        )
        .unwrap();
        assert!(load_page(dir.path(), "s", first.cursor.as_deref()).is_err());
        let page = load_page(dir.path(), "s", None).unwrap();
        assert!(page.items.is_empty());
        assert!(!page.has_more);
    }

    fn update_line(update: Value) -> String {
        json!({"method":"session/update","params":{"update":update}}).to_string()
    }

    fn write_update(file: &mut File, update: Value) -> u64 {
        let line = update_line(update);
        writeln!(file, "{line}").unwrap();
        line.len() as u64 + 1
    }

    #[test]
    fn large_history_bounds_interleaved_replacements_and_preserves_cursors() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(st::UPDATES_FILE);
        let mut file = File::create(&path).unwrap();
        for index in 0..10_000 {
            write_update(
                &mut file,
                json!({
                    "_meta":{"promptIndex":index},
                    "sessionUpdate":"user_message_chunk",
                    "content":{"type":"text","text":format!("m{index}")}
                }),
            );
        }
        write_update(
            &mut file,
            json!({
                "sessionUpdate":"session_info_update","title":"stress history"
            }),
        );
        write_update(
            &mut file,
            json!({"sessionUpdate":"usage_update","used":7,"size":11}),
        );
        writeln!(
            file,
            "{}",
            json!({"method":st::XAI_SESSION_UPDATE_METHOD,"params":{"update":{
                "sessionUpdate":"task_backgrounded","task_id":"orphan",
                "command":"sleep","cwd":"/tmp"
            }}})
        )
        .unwrap();
        write_update(
            &mut file,
            json!({
                "sessionUpdate":"tool_call","toolCallId":"tool",
                "title":"runner","status":"in_progress","rawInput":{}
            }),
        );
        for index in 0..1_000 {
            write_update(
                &mut file,
                json!({
                    "sessionUpdate":"agent_message_chunk",
                    "content":{"type":"text","text":"x"}
                }),
            );
            write_update(
                &mut file,
                json!({
                    "sessionUpdate":"tool_call_update","toolCallId":"tool",
                    "status":if index % 2 == 0 {"completed"} else {"in_progress"},
                    "content":[{"type":"content","content":{"type":"text","text":"x"}}]
                }),
            );
        }
        file.flush().unwrap();
        let migration_source_bytes = file.metadata().unwrap().len();
        let migration_started = std::time::Instant::now();
        let migrated = load_page(dir.path(), "stress", None).unwrap();
        let migration_millis = migration_started.elapsed().as_millis();
        assert!(migrated.migrated);
        assert_eq!(migrated.source_read_bytes, migration_source_bytes);
        assert_eq!(migrated.title.as_deref(), Some("stress history"));
        assert_eq!(migrated.usage, Some(json!({"used":7,"size":11})));

        let append_started = std::time::Instant::now();
        let mut appended_source_bytes = 0;
        for index in 1_000..10_000 {
            appended_source_bytes += write_update(
                &mut file,
                json!({
                    "sessionUpdate":"agent_message_chunk",
                    "content":{"type":"text","text":"x"}
                }),
            );
            appended_source_bytes += write_update(
                &mut file,
                json!({
                    "sessionUpdate":"tool_call_update","toolCallId":"tool",
                    "status":if index % 2 == 0 {"completed"} else {"in_progress"},
                    "content":[{"type":"content","content":{"type":"text","text":"x"}}]
                }),
            );
        }
        file.flush().unwrap();
        let appended = load_page(dir.path(), "stress", None).unwrap();
        let append_millis = append_started.elapsed().as_millis();
        assert!(!appended.migrated);
        assert_eq!(appended.source_read_bytes, appended_source_bytes);
        assert_eq!(appended.title.as_deref(), Some("stress history"));
        assert_eq!(appended.usage, Some(json!({"used":7,"size":11})));
        let meta_path = dir.path().join(INDEX_META);
        let meta = read_meta(&meta_path).unwrap();
        assert_eq!(meta.count, 10_002);
        assert_eq!(meta.orphaned_tasks.len(), 1);
        assert!(meta.data_len <= MAX_INDEX_BYTES as u64);
        assert!(meta.data_len <= meta.live_len * 17 + 64 * 1024);
        let old_cursor = appended.cursor.clone().unwrap();

        let mut pages = vec![appended];
        loop {
            let Some(cursor) = pages.last().unwrap().cursor.clone() else {
                break;
            };
            let page = load_page(dir.path(), "stress", Some(&cursor)).unwrap();
            assert!(!page.migrated);
            pages.push(page);
        }
        pages.reverse();
        let page_count = pages.len();
        let page_read_bytes: u64 = pages.iter().map(|page| page.read_bytes).sum();
        assert_eq!(page_count, 201);
        assert_eq!(page_read_bytes, meta.live_len + meta.count * 16);
        let all: Vec<_> = pages.into_iter().flat_map(|page| page.items).collect();
        assert_eq!(all.len(), 10_002);
        for (index, item) in all.iter().take(10_000).enumerate() {
            assert_eq!(item["text"], format!("m{index}"));
        }
        assert_eq!(all[10_000]["t"], "tool");
        assert_eq!(all[10_000]["status"], "in_progress");
        assert_eq!(all[10_001]["t"], "message.delta");
        assert_eq!(all[10_001]["text"].as_str().unwrap().len(), 10_000);

        let fresh_bytes = write_update(
            &mut file,
            json!({
                "_meta":{"promptIndex":10_000},
                "sessionUpdate":"user_message_chunk",
                "content":{"type":"text","text":"fresh"}
            }),
        );
        file.flush().unwrap();
        let older = load_page(dir.path(), "stress", Some(&old_cursor)).unwrap();
        assert!(!older.migrated);
        assert_eq!(older.source_read_bytes, fresh_bytes);
        assert_eq!(older.items.first().unwrap()["text"], "m9902");
        assert_eq!(older.items.last().unwrap()["text"], "m9951");
        let latest = load_page(dir.path(), "stress", None).unwrap();
        assert_eq!(latest.items.last().unwrap()["text"], "fresh");

        let tail = update_line(json!({
            "_meta":{"promptIndex":10_001},
            "sessionUpdate":"user_message_chunk",
            "content":{"type":"text","text":"finished tail"}
        }));
        let split = tail.len() / 2;
        file.write_all(&tail.as_bytes()[..split]).unwrap();
        file.flush().unwrap();
        let partial = load_page(dir.path(), "stress", None).unwrap();
        assert!(!partial.migrated);
        assert_eq!(partial.items.last().unwrap()["text"], "fresh");
        file.write_all(&tail.as_bytes()[split..]).unwrap();
        file.write_all(b"\n").unwrap();
        file.flush().unwrap();
        let completed = load_page(dir.path(), "stress", None).unwrap();
        assert!(!completed.migrated);
        assert_eq!(completed.items.last().unwrap()["text"], "finished tail");

        File::create(dir.path().join(INDEX_DATA))
            .unwrap()
            .write_all(b"partial index write")
            .unwrap();
        let rebuilt = load_page(dir.path(), "stress", None).unwrap();
        assert!(rebuilt.migrated);
        let rebuild_source_bytes = file.metadata().unwrap().len();
        assert_eq!(rebuilt.source_read_bytes, rebuild_source_bytes);
        assert_eq!(rebuilt.items.last().unwrap()["text"], "finished tail");
        let before_rewind_cursor = rebuilt.cursor.clone().unwrap();

        writeln!(
            file,
            "{}",
            json!({"method":st::XAI_SESSION_UPDATE_METHOD,"params":{"update":{
                "sessionUpdate":"rewind_marker","target_prompt_index":9_990
            }}})
        )
        .unwrap();
        file.flush().unwrap();
        assert!(load_page(dir.path(), "stress", Some(&before_rewind_cursor)).is_err());
        let rewound = load_page(dir.path(), "stress", None).unwrap();
        assert_eq!(rewound.items.last().unwrap()["text"], "m9989");
        assert_eq!(read_meta(&meta_path).unwrap().count, 9_990);

        eprintln!(
            "stress items={} pages={page_count} page_read_bytes={page_read_bytes} source_migration_bytes={migration_source_bytes} source_append_bytes={appended_source_bytes} source_warm_append_bytes={fresh_bytes} source_rebuild_bytes={rebuild_source_bytes} index_live_bytes={} index_physical_bytes={} migration_ms={migration_millis} append_ms={append_millis}",
            meta.count, meta.live_len, meta.data_len,
        );
    }

    #[test]
    fn history_migration_enforces_source_item_and_projection_size_caps() {
        let source_dir = tempfile::tempdir().unwrap();
        let source = File::create(source_dir.path().join(st::UPDATES_FILE)).unwrap();
        source.set_len(MAX_SOURCE_BYTES + 1).unwrap();
        let error = load_page(source_dir.path(), "source-cap", None)
            .err()
            .unwrap();
        assert!(error.to_string().contains("256 MiB"));

        let lines_dir = tempfile::tempdir().unwrap();
        std::fs::write(
            lines_dir.path().join(st::UPDATES_FILE),
            b"x\n".repeat(MAX_MIGRATION_LINES + 1),
        )
        .unwrap();
        let error = load_page(lines_dir.path(), "line-cap", None).err().unwrap();
        assert!(error.to_string().contains("1,000,000 lines"));

        let item_dir = tempfile::tempdir().unwrap();
        let mut item_source = File::create(item_dir.path().join(st::UPDATES_FILE)).unwrap();
        for index in 0..=MAX_INDEX_ITEMS {
            write_update(
                &mut item_source,
                json!({
                    "sessionUpdate":"user_message_chunk",
                    "content":{"type":"text","text":format!("item{index}")}
                }),
            );
        }
        let error = load_page(item_dir.path(), "item-cap", None).err().unwrap();
        assert!(error.to_string().contains("20,000 items"));

        let data_dir = tempfile::tempdir().unwrap();
        let mut data_source = File::create(data_dir.path().join(st::UPDATES_FILE)).unwrap();
        let text = "x".repeat(MAX_INDEX_BYTES + 1);
        write_update(
            &mut data_source,
            json!({
                "sessionUpdate":"user_message_chunk",
                "content":{"type":"text","text":text}
            }),
        );
        let error = load_page(data_dir.path(), "data-cap", None).err().unwrap();
        assert!(error.to_string().contains("32 MiB"));
    }

    #[test]
    fn streaming_replacements_do_not_accumulate_obsolete_text() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(st::UPDATES_FILE);
        let delta = json!({"method":"session/update","params":{"update":{
            "sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"x"}
        }}})
        .to_string();
        let mut file = File::create(&path).unwrap();
        for _ in 0..8192 {
            writeln!(file, "{delta}").unwrap();
        }
        let initial = load_page(dir.path(), "s", None).unwrap();
        assert_eq!(initial.items[0]["text"].as_str().unwrap().len(), 8192);
        assert!(read_meta(&dir.path().join(INDEX_META)).unwrap().data_len < 9000);
        for _ in 0..1024 {
            writeln!(file, "{delta}").unwrap();
        }
        let appended = load_page(dir.path(), "s", None).unwrap();
        assert!(!appended.migrated);
        assert_eq!(appended.items[0]["text"].as_str().unwrap().len(), 9216);
        assert!(read_meta(&dir.path().join(INDEX_META)).unwrap().data_len < 10000);
    }
}
