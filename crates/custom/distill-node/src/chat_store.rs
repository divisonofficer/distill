use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use crate::sep::TextFileIn;
use anyhow::{Context, Result, anyhow, bail};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ring::rand::{SecureRandom, SystemRandom};
use ring::signature::{ED25519, Ed25519KeyPair, KeyPair, UnparsedPublicKey};
use serde::{Deserialize, Serialize};

const KEY_FILE: &str = "chat-store.key";
const SNAPSHOT_FILE: &str = "chat-store.json";
const SNAPSHOT_VERSION: u32 = 1;
const MAX_ID_BYTES: usize = 128;
pub const MAX_TEXT_BYTES: usize = 1024 * 1024;
pub const MAX_TITLE_BYTES: usize = 300;
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    pub node_id: String,
    pub public_key: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChatEvent {
    Create {
        chat_id: String,
        title: String,
    },
    Message {
        chat_id: String,
        message_id: String,
        parent_id: Option<String>,
        role: String,
        text: String,
        #[serde(default)]
        model: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        text_files: Vec<TextFileIn>,
    },
    Title {
        chat_id: String,
        title: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Envelope {
    pub author: String,
    pub seq: u64,
    pub previous_hash: Option<String>,
    pub event: ChatEvent,
    pub signature: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub message_id: String,
    pub parent_id: Option<String>,
    pub role: String,
    pub text: String,
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub text_files: Vec<TextFileIn>,
    pub author: String,
    pub seq: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Conversation {
    pub chat_id: String,
    pub title: Option<String>,
    pub messages: Vec<ChatMessage>,
}

#[derive(Clone)]
pub struct Store {
    inner: Arc<StoreInner>,
}

struct StoreInner {
    dir: PathBuf,
    key_seed: [u8; 32],
    identity: Identity,
    state: Mutex<State>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum CompletionClaimStatus {
    Pending,
    Retryable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct CompletionClaim {
    chat_id: String,
    message_id: String,
    user_author: String,
    model_fingerprint: String,
    status: CompletionClaimStatus,
}

#[derive(Clone, Default)]
struct State {
    events: BTreeMap<String, BTreeMap<u64, Envelope>>,
    trusted_keys: BTreeMap<String, String>,
    completion_claims: Vec<CompletionClaim>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    version: u32,
    events: Vec<Envelope>,
    trusted_keys: BTreeMap<String, String>,
    #[serde(default)]
    completion_claims: Vec<CompletionClaim>,
}

impl Store {
    /// One process must own each store directory; synchronization is in-process only.
    pub fn open(dir: &Path) -> Result<Self> {
        create_private_dir(dir).with_context(|| format!("create chat store {}", dir.display()))?;
        let key_path = dir.join(KEY_FILE);
        let key_seed = load_or_create_seed(&key_path)?;
        let key = key_pair(&key_seed)?;
        let public_key = URL_SAFE_NO_PAD.encode(key.public_key().as_ref());
        let identity = Identity {
            node_id: fingerprint(key.public_key().as_ref()),
            public_key,
        };
        let state = load_state(dir, &identity)?;

        Ok(Self {
            inner: Arc::new(StoreInner {
                dir: dir.to_path_buf(),
                key_seed,
                identity,
                state: Mutex::new(state),
            }),
        })
    }

    pub fn identity(&self) -> Identity {
        self.inner.identity.clone()
    }

    pub fn append(&self, event: ChatEvent) -> Result<Envelope> {
        validate_event(&event)?;
        let mut state = self.lock();
        let author = &self.inner.identity.node_id;
        let author_events = state.events.get(author);
        let (seq, previous_hash) = match author_events.and_then(|chain| chain.last_key_value()) {
            Some((seq, previous)) => (
                seq.checked_add(1)
                    .ok_or_else(|| anyhow!("author sequence overflow"))?,
                Some(envelope_hash(previous)?),
            ),
            None => (1, None),
        };
        let key = key_pair(&self.inner.key_seed)?;
        let mut envelope = Envelope {
            author: author.clone(),
            seq,
            previous_hash,
            event,
            signature: String::new(),
        };
        envelope.signature = URL_SAFE_NO_PAD.encode(key.sign(&signing_bytes(&envelope)?).as_ref());

        let mut candidate = state.clone();
        candidate
            .events
            .entry(author.clone())
            .or_default()
            .insert(seq, envelope.clone());
        candidate
            .trusted_keys
            .insert(author.clone(), self.inner.identity.public_key.clone());
        self.persist(&candidate)?;
        *state = candidate;
        Ok(envelope)
    }

    /// Import only authors explicitly trusted by this call; event chains may be batched out of order.
    pub fn import(
        &self,
        events: Vec<Envelope>,
        trusted_keys: &BTreeMap<String, String>,
    ) -> Result<usize> {
        if events.is_empty() {
            return Ok(0);
        }

        let mut state = self.lock();
        let mut candidate = state.clone();
        let mut added = 0;
        for envelope in events {
            validate_event(&envelope.event)?;
            if envelope.seq == 0 {
                bail!("author sequence must start at one");
            }
            let public_key = trusted_keys
                .get(&envelope.author)
                .ok_or_else(|| anyhow!("untrusted author {}", envelope.author))?;
            validate_public_key(&envelope.author, public_key)?;
            if let Some(existing_key) = candidate.trusted_keys.get(&envelope.author)
                && existing_key != public_key
            {
                bail!("author key changed for {}", envelope.author);
            }
            candidate
                .trusted_keys
                .insert(envelope.author.clone(), public_key.clone());

            match candidate
                .events
                .entry(envelope.author.clone())
                .or_default()
                .get(&envelope.seq)
            {
                Some(existing) if existing == &envelope => {}
                Some(_) => bail!(
                    "author sequence fork for {}:{}",
                    envelope.author,
                    envelope.seq
                ),
                None => {
                    candidate
                        .events
                        .get_mut(&envelope.author)
                        .expect("author chain inserted")
                        .insert(envelope.seq, envelope);
                    added += 1;
                }
            }
        }
        validate_state(&candidate)?;
        if added == 0 && candidate.trusted_keys == state.trusted_keys {
            return Ok(0);
        }
        self.persist(&candidate)?;
        *state = candidate;
        Ok(added)
    }

    pub fn export(&self) -> Vec<Envelope> {
        flatten_events(&self.lock())
    }

    pub fn conversations(&self) -> Vec<Conversation> {
        conversations_from_events(&flatten_events(&self.lock()))
    }

    pub fn conversation(&self, id: &str) -> Option<Conversation> {
        conversation_from_state(&self.lock(), id)
    }

    pub fn claim_completion(
        &self,
        chat_id: &str,
        message_id: &str,
        user_author: &str,
        model_id: &str,
    ) -> Result<bool> {
        validate_id(chat_id, "chat_id")?;
        validate_id(message_id, "message_id")?;
        validate_id(user_author, "user_author")?;
        if model_id.trim().is_empty() || model_id.len() > 1024 {
            bail!("invalid completion model id");
        }
        let model_fingerprint = completion_model_fingerprint(model_id);
        let mut state = self.lock();
        let target_is_user_message =
            conversation_from_state(&state, chat_id).is_some_and(|conversation| {
                conversation.messages.iter().any(|message| {
                    message.message_id == message_id
                        && message.author == user_author
                        && message.role == "user"
                })
            });
        if !target_is_user_message {
            bail!("completion claim target is not a user message");
        }
        let existing = state.completion_claims.iter().position(|claim| {
            claim.chat_id == chat_id
                && claim.message_id == message_id
                && claim.user_author == user_author
                && claim.model_fingerprint == model_fingerprint
        });
        if existing.is_some_and(|index| {
            state.completion_claims[index].status == CompletionClaimStatus::Pending
        }) {
            return Ok(false);
        }

        let mut candidate = state.clone();
        if let Some(index) = existing {
            candidate.completion_claims[index].status = CompletionClaimStatus::Pending;
        } else {
            candidate.completion_claims.push(CompletionClaim {
                chat_id: chat_id.to_owned(),
                message_id: message_id.to_owned(),
                user_author: user_author.to_owned(),
                model_fingerprint,
                status: CompletionClaimStatus::Pending,
            });
        }
        self.persist(&candidate)?;
        *state = candidate;
        Ok(true)
    }

    pub fn mark_completion_retryable(
        &self,
        chat_id: &str,
        message_id: &str,
        user_author: &str,
        model_id: &str,
    ) -> Result<()> {
        let model_fingerprint = completion_model_fingerprint(model_id);
        let mut state = self.lock();
        let mut candidate = state.clone();
        let Some(claim) = candidate.completion_claims.iter_mut().find(|claim| {
            claim.chat_id == chat_id
                && claim.message_id == message_id
                && claim.user_author == user_author
                && claim.model_fingerprint == model_fingerprint
        }) else {
            bail!("completion claim is missing");
        };
        if claim.status != CompletionClaimStatus::Pending {
            bail!("completion claim is not pending");
        }
        claim.status = CompletionClaimStatus::Retryable;
        self.persist(&candidate)?;
        *state = candidate;
        Ok(())
    }

    fn persist(&self, state: &State) -> Result<()> {
        let snapshot = Snapshot {
            version: SNAPSHOT_VERSION,
            events: flatten_events(state),
            trusted_keys: state.trusted_keys.clone(),
            completion_claims: state.completion_claims.clone(),
        };
        let bytes = serde_json::to_vec(&snapshot).context("serialize chat-store snapshot")?;
        write_snapshot(&self.inner.dir.join(SNAPSHOT_FILE), &bytes)
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

fn validate_event(event: &ChatEvent) -> Result<()> {
    match event {
        ChatEvent::Create { chat_id, title } | ChatEvent::Title { chat_id, title } => {
            validate_id(chat_id, "chat_id")?;
            validate_title(title)
        }
        ChatEvent::Message {
            chat_id,
            message_id,
            parent_id,
            role,
            text,
            text_files,
            ..
        } => {
            validate_id(chat_id, "chat_id")?;
            validate_id(message_id, "message_id")?;
            if let Some(parent_id) = parent_id {
                validate_id(parent_id, "parent_id")?;
            }
            if role != "user" && role != "assistant" {
                bail!("unsupported message role");
            }
            if text.len() > MAX_TEXT_BYTES {
                bail!("message text exceeds 1 MiB");
            }
            if role == "assistant" && !text_files.is_empty() {
                bail!("assistant messages cannot contain text files");
            }
            let normalized =
                crate::attachments::chat_text_files(text_files).map_err(|error| anyhow!(error))?;
            if &normalized != text_files {
                bail!("text file names must be canonical basenames");
            }
            Ok(())
        }
    }
}

fn validate_id(id: &str, field: &str) -> Result<()> {
    if id.trim().is_empty() {
        bail!("{field} must not be empty");
    }
    if id.len() > MAX_ID_BYTES {
        bail!("{field} exceeds 128 bytes");
    }
    Ok(())
}

fn validate_title(title: &str) -> Result<()> {
    if title.len() > MAX_TITLE_BYTES {
        bail!("title exceeds 300 bytes");
    }
    Ok(())
}

fn validate_public_key(author: &str, encoded_key: &str) -> Result<Vec<u8>> {
    let public_key = URL_SAFE_NO_PAD
        .decode(encoded_key)
        .context("decode Ed25519 public key")?;
    if URL_SAFE_NO_PAD.encode(&public_key) != encoded_key {
        bail!("non-canonical Ed25519 public key encoding");
    }
    if public_key.len() != 32 || fingerprint(&public_key) != author {
        bail!("public key does not identify author {author}");
    }
    Ok(public_key)
}

fn signing_bytes(envelope: &Envelope) -> Result<Vec<u8>> {
    serde_json::to_vec(&(
        envelope.author.as_str(),
        envelope.seq,
        envelope.previous_hash.as_deref(),
        &envelope.event,
    ))
    .context("serialize signed chat event")
}

fn envelope_hash(envelope: &Envelope) -> Result<String> {
    Ok(hex(ring::digest::digest(
        &ring::digest::SHA256,
        &serde_json::to_vec(envelope).context("serialize chat envelope")?,
    )
    .as_ref()))
}

fn fingerprint(public_key: &[u8]) -> String {
    hex(ring::digest::digest(&ring::digest::SHA256, public_key).as_ref())
}

fn completion_model_fingerprint(model_id: &str) -> String {
    hex(ring::digest::digest(&ring::digest::SHA256, model_id.as_bytes()).as_ref())
}

fn hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}").expect("write to string");
    }
    output
}

fn flatten_events(state: &State) -> Vec<Envelope> {
    state
        .events
        .values()
        .flat_map(|chain| chain.values().cloned())
        .collect()
}

fn conversation_from_state(state: &State, chat_id: &str) -> Option<Conversation> {
    let mut conversation = Conversation {
        chat_id: chat_id.to_owned(),
        title: None,
        messages: Vec::new(),
    };
    let mut found = false;
    let mut has_explicit_title = false;
    for envelope in state.events.values().flat_map(|chain| chain.values()) {
        match &envelope.event {
            ChatEvent::Create { chat_id: id, title } if id == chat_id => {
                conversation.title = Some(title.clone());
                found = true;
            }
            ChatEvent::Title { chat_id: id, title } if id == chat_id => {
                conversation.title = Some(title.clone());
                has_explicit_title = true;
                found = true;
            }
            ChatEvent::Message {
                chat_id: id,
                message_id,
                parent_id,
                role,
                text,
                model,
                text_files,
            } if id == chat_id => {
                conversation.messages.push(ChatMessage {
                    message_id: message_id.clone(),
                    parent_id: parent_id.clone(),
                    role: role.clone(),
                    text: text.clone(),
                    model: model.clone(),
                    text_files: text_files.clone(),
                    author: envelope.author.clone(),
                    seq: envelope.seq,
                });
                found = true;
            }
            _ => {}
        }
    }
    if !has_explicit_title {
        apply_default_title(&mut conversation);
    }
    found.then_some(conversation)
}

fn conversations_from_events(events: &[Envelope]) -> Vec<Conversation> {
    let mut conversations = BTreeMap::<String, Conversation>::new();
    let mut explicit_title_ids = BTreeSet::new();
    for envelope in events {
        let (chat_id, event) = match &envelope.event {
            ChatEvent::Create { chat_id, .. }
            | ChatEvent::Message { chat_id, .. }
            | ChatEvent::Title { chat_id, .. } => (chat_id, &envelope.event),
        };
        let conversation = conversations
            .entry(chat_id.clone())
            .or_insert_with(|| Conversation {
                chat_id: chat_id.clone(),
                title: None,
                messages: Vec::new(),
            });
        match event {
            ChatEvent::Create { title, .. } => {
                conversation.title = Some(title.clone());
            }
            ChatEvent::Title { chat_id, title } => {
                conversation.title = Some(title.clone());
                explicit_title_ids.insert(chat_id.clone());
            }
            ChatEvent::Message {
                message_id,
                parent_id,
                role,
                text,
                model,
                text_files,
                ..
            } => conversation.messages.push(ChatMessage {
                message_id: message_id.clone(),
                parent_id: parent_id.clone(),
                role: role.clone(),
                text: text.clone(),
                model: model.clone(),
                text_files: text_files.clone(),
                author: envelope.author.clone(),
                seq: envelope.seq,
            }),
        }
    }
    for conversation in conversations.values_mut() {
        if !explicit_title_ids.contains(&conversation.chat_id) {
            apply_default_title(conversation);
        }
    }
    conversations.into_values().collect()
}

fn apply_default_title(conversation: &mut Conversation) {
    if conversation.title.as_deref() != Some("New chat") {
        return;
    }
    let Some(first_user_text) = conversation
        .messages
        .iter()
        .find(|message| message.role == "user" && !message.text.trim().is_empty())
        .map(|message| message.text.as_str())
    else {
        return;
    };
    let normalized = first_user_text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let mut chars = normalized.chars();
    let short_title = chars.by_ref().take(64).collect::<String>();
    let title = if chars.next().is_some() {
        format!("{short_title}…")
    } else {
        short_title
    };
    if !title.is_empty() {
        conversation.title = Some(title);
    }
}

fn load_state(dir: &Path, identity: &Identity) -> Result<State> {
    let path = dir.join(SNAPSHOT_FILE);
    let mut state = match fs::symlink_metadata(&path) {
        Ok(metadata) => {
            if !metadata.file_type().is_file() {
                bail!("chat-store snapshot is not a regular file");
            }
            set_private_file(&path)?;
            let bytes = fs::read(&path).context("read chat-store snapshot")?;
            let snapshot: Snapshot =
                serde_json::from_slice(&bytes).context("parse chat-store snapshot")?;
            if snapshot.version != SNAPSHOT_VERSION {
                bail!(
                    "unsupported chat-store snapshot version {}",
                    snapshot.version
                );
            }
            state_from_snapshot(snapshot)?
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => State::default(),
        Err(error) => return Err(error).context("inspect chat-store snapshot"),
    };
    match state.trusted_keys.get(&identity.node_id) {
        Some(public_key) if public_key != &identity.public_key => {
            bail!("stored local author key does not match identity key");
        }
        Some(_) => {}
        None => {
            state
                .trusted_keys
                .insert(identity.node_id.clone(), identity.public_key.clone());
        }
    }
    Ok(state)
}

fn state_from_snapshot(snapshot: Snapshot) -> Result<State> {
    let Snapshot {
        events,
        trusted_keys,
        completion_claims,
        ..
    } = snapshot;
    let mut state = State {
        events: BTreeMap::new(),
        trusted_keys,
        completion_claims,
    };
    for (author, public_key) in &state.trusted_keys {
        validate_public_key(author, public_key)?;
    }
    for envelope in events {
        validate_event(&envelope.event)?;
        let chain = state.events.entry(envelope.author.clone()).or_default();
        if chain.insert(envelope.seq, envelope).is_some() {
            bail!("duplicate author sequence in chat-store snapshot");
        }
    }
    validate_state(&state)?;
    Ok(state)
}

fn validate_state(state: &State) -> Result<()> {
    for (author, chain) in &state.events {
        let encoded_key = state
            .trusted_keys
            .get(author)
            .ok_or_else(|| anyhow!("missing trusted key for author {author}"))?;
        let public_key = validate_public_key(author, encoded_key)?;
        let mut expected_seq = 1_u64;
        let mut previous_hash = None;
        for (seq, envelope) in chain {
            if *seq != expected_seq || envelope.seq != *seq || envelope.author != *author {
                bail!("non-contiguous event chain for author {author}");
            }
            if envelope.previous_hash != previous_hash {
                bail!("previous hash mismatch for author {author}:{seq}");
            }
            let signature = URL_SAFE_NO_PAD
                .decode(&envelope.signature)
                .context("decode event signature")?;
            UnparsedPublicKey::new(&ED25519, &public_key)
                .verify(&signing_bytes(envelope)?, &signature)
                .map_err(|_| anyhow!("invalid event signature for {author}:{seq}"))?;
            previous_hash = Some(envelope_hash(envelope)?);
            expected_seq = expected_seq
                .checked_add(1)
                .ok_or_else(|| anyhow!("author sequence overflow"))?;
        }
    }
    let mut claim_keys = BTreeSet::new();
    for claim in &state.completion_claims {
        validate_id(&claim.chat_id, "chat_id")?;
        validate_id(&claim.message_id, "message_id")?;
        validate_id(&claim.user_author, "user_author")?;
        if claim.model_fingerprint.len() != 64
            || !claim
                .model_fingerprint
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            bail!("invalid completion model fingerprint");
        }
        if !claim_keys.insert((
            claim.chat_id.as_str(),
            claim.message_id.as_str(),
            claim.user_author.as_str(),
            claim.model_fingerprint.as_str(),
        )) {
            bail!("duplicate local completion claim");
        }
        let target_is_user_message = state.events.get(&claim.user_author).is_some_and(|chain| {
            chain.values().any(|envelope| {
                matches!(
                    &envelope.event,
                    ChatEvent::Message {
                        chat_id,
                        message_id,
                        role,
                        ..
                    } if chat_id == &claim.chat_id
                        && message_id == &claim.message_id
                        && role == "user"
                )
            })
        });
        if !target_is_user_message {
            bail!("local completion claim has no matching user message");
        }
    }
    Ok(())
}

fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    match builder.create(dir) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            if !fs::symlink_metadata(dir)?.file_type().is_dir() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::NotADirectory,
                    "chat store path is not a directory",
                ));
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
            }
            Ok(())
        }
        Err(error) => Err(error),
    }
}

fn load_or_create_seed(path: &Path) -> Result<[u8; 32]> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if !metadata.file_type().is_file() {
                bail!("chat-store key is not a regular file");
            }
            set_private_file(path)?;
            read_seed(path)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut seed = [0_u8; 32];
            SystemRandom::new()
                .fill(&mut seed)
                .map_err(|_| anyhow!("generate chat-store identity key"))?;
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            match options.open(path) {
                Ok(mut file) => {
                    file.write_all(&seed)
                        .context("write chat-store identity key")?;
                    file.sync_all().context("sync chat-store identity key")?;
                    sync_directory(path.parent().expect("key path has parent"))?;
                    Ok(seed)
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => read_seed(path),
                Err(error) => Err(error).context("create chat-store identity key"),
            }
        }
        Err(error) => Err(error).context("inspect chat-store identity key"),
    }
}

fn read_seed(path: &Path) -> Result<[u8; 32]> {
    let bytes = fs::read(path).context("read chat-store identity key")?;
    bytes
        .try_into()
        .map_err(|_| anyhow!("chat-store identity key must be 32 bytes"))
}

fn key_pair(seed: &[u8; 32]) -> Result<Ed25519KeyPair> {
    Ed25519KeyPair::from_seed_unchecked(seed).map_err(|_| anyhow!("load Ed25519 identity key"))
}

fn set_private_file(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn write_snapshot(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path.parent().expect("snapshot path has parent");
    let (temp_path, mut temp_file) = create_temp_file(dir)?;
    let write_result = temp_file
        .write_all(bytes)
        .and_then(|()| temp_file.sync_all());
    drop(temp_file);
    if let Err(error) = write_result {
        let _ = fs::remove_file(&temp_path);
        return Err(error).context("write chat-store snapshot");
    }
    if let Err(error) = fs::rename(&temp_path, path) {
        let _ = fs::remove_file(&temp_path);
        return Err(error).context("replace chat-store snapshot");
    }
    sync_directory(dir).context("sync chat-store directory")?;
    Ok(())
}

fn create_temp_file(dir: &Path) -> Result<(PathBuf, File)> {
    for _ in 0..100 {
        let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = dir.join(format!(
            ".{SNAPSHOT_FILE}.{}.{}.tmp",
            std::process::id(),
            counter
        ));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error).context("create temporary chat-store snapshot"),
        }
    }
    bail!("could not allocate temporary chat-store snapshot")
}

fn sync_directory(dir: &Path) -> std::io::Result<()> {
    File::open(dir)?.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_event(chat_id: &str) -> ChatEvent {
        ChatEvent::Create {
            chat_id: chat_id.into(),
            title: "Chat".into(),
        }
    }

    fn message_event(message_id: &str, parent_id: Option<&str>, role: &str) -> ChatEvent {
        ChatEvent::Message {
            chat_id: "chat".into(),
            message_id: message_id.into(),
            parent_id: parent_id.map(str::to_owned),
            role: role.into(),
            text: message_id.into(),
            model: None,
            text_files: Vec::new(),
        }
    }

    fn trusted(store: &Store) -> BTreeMap<String, String> {
        let identity = store.identity();
        BTreeMap::from([(identity.node_id, identity.public_key)])
    }

    fn signed_envelope(
        store: &Store,
        seq: u64,
        previous: Option<String>,
        event: ChatEvent,
    ) -> Envelope {
        let identity = store.identity();
        let seed = read_seed(&store.inner.dir.join(KEY_FILE)).unwrap();
        let key = key_pair(&seed).unwrap();
        let mut envelope = Envelope {
            author: identity.node_id,
            seq,
            previous_hash: previous,
            event,
            signature: String::new(),
        };
        envelope.signature =
            URL_SAFE_NO_PAD.encode(key.sign(&signing_bytes(&envelope).unwrap()).as_ref());
        envelope
    }

    #[test]
    fn append_and_reopen_preserves_exact_events() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let identity = store.identity();
        store.append(create_event("chat")).unwrap();
        store.append(message_event("first", None, "user")).unwrap();
        let expected = store.export();
        drop(store);

        let reopened = Store::open(dir.path()).unwrap();
        assert_eq!(reopened.identity(), identity);
        assert_eq!(reopened.export(), expected);
        assert_eq!(
            reopened.conversation("chat").unwrap().messages[0].message_id,
            "first"
        );
    }

    #[test]
    fn imports_repeated_reverse_order_batches_and_preserves_author_chain() {
        let source_dir = tempfile::tempdir().unwrap();
        let sink_dir = tempfile::tempdir().unwrap();
        let source = Store::open(source_dir.path()).unwrap();
        let sink = Store::open(sink_dir.path()).unwrap();
        source.append(create_event("chat")).unwrap();
        source.append(message_event("first", None, "user")).unwrap();
        let mut reverse = source.export();
        reverse.reverse();

        assert_eq!(sink.import(reverse.clone(), &trusted(&source)).unwrap(), 2);
        assert_eq!(sink.import(reverse, &trusted(&source)).unwrap(), 0);
        assert_eq!(sink.export(), source.export());
    }

    #[test]
    fn offline_sibling_messages_remain_separate_branches() {
        let left_dir = tempfile::tempdir().unwrap();
        let right_dir = tempfile::tempdir().unwrap();
        let left = Store::open(left_dir.path()).unwrap();
        let right = Store::open(right_dir.path()).unwrap();
        left.append(create_event("chat")).unwrap();
        left.append(message_event("parent", None, "user")).unwrap();
        right.import(left.export(), &trusted(&left)).unwrap();
        left.append(message_event("left-child", Some("parent"), "assistant"))
            .unwrap();
        right
            .append(message_event("right-child", Some("parent"), "assistant"))
            .unwrap();

        let left_identity = left.identity();
        let right_identity = right.identity();
        let all_keys = BTreeMap::from([
            (left_identity.node_id, left_identity.public_key),
            (right_identity.node_id, right_identity.public_key),
        ]);
        left.import(right.export(), &all_keys).unwrap();
        right.import(left.export(), &all_keys).unwrap();
        for store in [&left, &right] {
            let messages = &store.conversation("chat").unwrap().messages;
            let children = messages
                .iter()
                .filter(|message| message.parent_id.as_deref() == Some("parent"))
                .map(|message| message.message_id.as_str())
                .collect::<Vec<_>>();
            assert_eq!(children.len(), 2);
            assert!(children.contains(&"left-child"));
            assert!(children.contains(&"right-child"));
        }
    }

    #[test]
    fn rejects_untrusted_bad_signature_and_author_fork() {
        let source_dir = tempfile::tempdir().unwrap();
        let sink_dir = tempfile::tempdir().unwrap();
        let source = Store::open(source_dir.path()).unwrap();
        let sink = Store::open(sink_dir.path()).unwrap();
        let original = source.append(create_event("chat")).unwrap();
        let keys = trusted(&source);

        assert!(
            sink.import(vec![original.clone()], &BTreeMap::new())
                .is_err()
        );
        let mut bad_signature = original.clone();
        bad_signature.signature = URL_SAFE_NO_PAD.encode([0_u8; 64]);
        assert!(sink.import(vec![bad_signature], &keys).is_err());
        assert!(sink.export().is_empty());

        sink.import(vec![original.clone()], &keys).unwrap();
        let fork = signed_envelope(&source, 1, None, create_event("different"));
        assert!(sink.import(vec![fork], &keys).is_err());
        assert_eq!(sink.export(), vec![original]);
    }

    #[test]
    fn malformed_batch_rolls_back_and_rejects_unsupported_role() {
        let source_dir = tempfile::tempdir().unwrap();
        let sink_dir = tempfile::tempdir().unwrap();
        let source = Store::open(source_dir.path()).unwrap();
        let sink = Store::open(sink_dir.path()).unwrap();
        let first = source.append(create_event("chat")).unwrap();
        let second = signed_envelope(
            &source,
            2,
            Some(envelope_hash(&first).unwrap()),
            message_event("bad-role", None, "tool"),
        );
        assert!(sink.import(vec![first, second], &trusted(&source)).is_err());
        assert!(sink.export().is_empty());
        assert!(
            source
                .append(message_event("bad-role", None, "tool"))
                .is_err()
        );
    }

    #[test]
    fn enforces_text_and_id_size_limits() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let mut event = message_event("m", None, "user");
        if let ChatEvent::Message { text, .. } = &mut event {
            text.push_str(&"x".repeat(MAX_TEXT_BYTES - text.len()));
        }
        store.append(event).unwrap();
        assert!(
            store
                .append(message_event(&"i".repeat(129), None, "user"))
                .is_err()
        );
        let mut too_long = message_event("m2", None, "assistant");
        if let ChatEvent::Message { text, .. } = &mut too_long {
            *text = "x".repeat(MAX_TEXT_BYTES + 1);
        }
        assert!(store.append(too_long).is_err());
    }

    #[test]
    fn corrupt_snapshot_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        store.append(create_event("chat")).unwrap();
        fs::write(dir.path().join(SNAPSHOT_FILE), b"not-json").unwrap();
        assert!(Store::open(dir.path()).is_err());
    }
}
