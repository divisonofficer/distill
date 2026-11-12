use std::collections::{HashMap, HashSet, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ring::hmac;
use ring::rand::{SecureRandom, SystemRandom};
use serde::de::{DeserializeOwned, Deserializer};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, watch};

use crate::agent::{RequestFailure, lock as agent_lock};
use crate::chat_store::{
    ChatEvent, ChatMessage, Conversation, MAX_TEXT_BYTES, MAX_TITLE_BYTES, Store,
};
use crate::chat_sync::{SyncHealth, SyncStatus};
use crate::keeper::{Keeper, MAX_CHAT_PROGRESS_BYTES};

const MAX_REQUEST_BODY_BYTES: usize = 6 * 1024 * 1024 + 1024;
const HISTORY_PAGE_SIZE: usize = 50;
const MAX_HISTORY_CURSOR_BYTES: usize = 4096;
const ID_RANDOM_BYTES: usize = 18;
const ID_ATTEMPTS: usize = 8;
const COMPLETION_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const MAX_MODEL_ID_BYTES: usize = 1024;
const FAILED_COMPLETION_CAPACITY: usize = 128;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionPromptMessage {
    pub role: String,
    pub text: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionResult {
    pub model_id: String,
    pub text: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompletionError {
    /// Provider call provably did not dispatch; stored claim may safely be retried.
    Definitive,
    /// Provider may have received the call, or its result could not be validated.
    OutcomeUnknown,
}

pub trait ChatCompleter: Send + Sync {
    fn complete(
        &self,
        model_id: String,
        messages: Vec<CompletionPromptMessage>,
    ) -> Pin<Box<dyn Future<Output = Result<CompletionResult, CompletionError>> + Send>>;

    fn complete_with_progress(
        &self,
        model_id: String,
        messages: Vec<CompletionPromptMessage>,
        _progress: watch::Sender<String>,
    ) -> Pin<Box<dyn Future<Output = Result<CompletionResult, CompletionError>> + Send>> {
        self.complete(model_id, messages)
    }
}

pub struct AcpChatCompleter {
    keeper: Keeper,
}

impl AcpChatCompleter {
    pub fn new(keeper: Keeper) -> Self {
        Self { keeper }
    }
}

impl ChatCompleter for AcpChatCompleter {
    fn complete(
        &self,
        model_id: String,
        messages: Vec<CompletionPromptMessage>,
    ) -> Pin<Box<dyn Future<Output = Result<CompletionResult, CompletionError>> + Send>> {
        let keeper = self.keeper.clone();
        Box::pin(async move {
            let params = serde_json::json!({"modelId": model_id, "messages": messages});
            let result = keeper
                .handle()
                .request_with_timeout_classified("_x.ai/chat/complete", params, COMPLETION_TIMEOUT)
                .await;
            decode_acp_completion(result)
        })
    }

    fn complete_with_progress(
        &self,
        model_id: String,
        messages: Vec<CompletionPromptMessage>,
        progress: watch::Sender<String>,
    ) -> Pin<Box<dyn Future<Output = Result<CompletionResult, CompletionError>> + Send>> {
        let keeper = self.keeper.clone();
        Box::pin(async move {
            let mut notifications = keeper.chat_progress();
            let Some(request_id) = new_id() else {
                return Err(CompletionError::Definitive);
            };
            let params = serde_json::json!({
                "modelId": model_id,
                "messages": messages,
                "requestId": request_id,
            });
            let handle = keeper.handle();
            let request = handle.request_with_timeout_classified(
                "_x.ai/chat/complete",
                params,
                COMPLETION_TIMEOUT,
            );
            tokio::pin!(request);
            loop {
                tokio::select! {
                    result = &mut request => return decode_acp_completion(result),
                    notification = notifications.recv() => match notification {
                        Ok(notification)
                            if notification.request_id == request_id
                                && notification.text.len() <= MAX_CHAT_PROGRESS_BYTES =>
                        {
                            progress.send_replace(notification.text);
                        }
                        Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
            }
            decode_acp_completion(request.await)
        })
    }
}

fn decode_acp_completion(
    result: Result<serde_json::Value, RequestFailure>,
) -> Result<CompletionResult, CompletionError> {
    let result = result.map_err(|_| CompletionError::OutcomeUnknown)?;
    serde_json::from_value(result).map_err(|_| CompletionError::OutcomeUnknown)
}

#[derive(Clone)]
struct ChatState {
    store: Store,
    history_cursor_key: Arc<hmac::Key>,
    completer: Option<Arc<dyn ChatCompleter>>,
    sync_status: SyncStatus,
    pending: Arc<Mutex<HashMap<(String, String), CompletionState>>>,
    progress: Arc<Mutex<HashMap<(String, String), CompletionProgress>>>,
    failures: Arc<Mutex<VecDeque<FailedCompletion>>>,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum CompletionState {
    Pending,
    Unknown,
}

#[derive(Clone)]
struct CompletionProgress {
    model_id: String,
    text: watch::Receiver<String>,
}

struct FailedCompletion {
    key: (String, String),
    model_id: String,
}

struct CompletionGate {
    pending: Arc<Mutex<HashMap<(String, String), CompletionState>>>,
    progress: Arc<Mutex<HashMap<(String, String), CompletionProgress>>>,
    failures: Arc<Mutex<VecDeque<FailedCompletion>>>,
    key: (String, String),
    completed: bool,
}

impl CompletionGate {
    fn release(mut self) {
        self.clear();
        self.completed = true;
    }

    fn fail(mut self, model_id: String) {
        {
            let mut failures = agent_lock(&self.failures);
            failures.retain(|failure| failure.key != self.key || failure.model_id != model_id);
            failures.push_back(FailedCompletion {
                key: self.key.clone(),
                model_id,
            });
            while failures.len() > FAILED_COMPLETION_CAPACITY {
                failures.pop_front();
            }
        }
        self.clear();
        self.completed = true;
    }

    fn mark_unknown(mut self) {
        {
            let mut pending = agent_lock(&self.pending);
            let mut progress = agent_lock(&self.progress);
            pending.insert(self.key.clone(), CompletionState::Unknown);
            progress.remove(&self.key);
        }
        self.completed = true;
    }

    fn clear(&self) {
        let mut pending = agent_lock(&self.pending);
        let mut progress = agent_lock(&self.progress);
        pending.remove(&self.key);
        progress.remove(&self.key);
    }
}

impl Drop for CompletionGate {
    fn drop(&mut self) {
        if !self.completed {
            let mut pending = agent_lock(&self.pending);
            let mut progress = agent_lock(&self.progress);
            pending.insert(self.key.clone(), CompletionState::Unknown);
            progress.remove(&self.key);
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct CompletionRequest {
    model_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProgressQuery {
    model_id: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MessageProgressResponse {
    status: &'static str,
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    model_id: Option<String>,
}

#[derive(Serialize)]
struct CompletionResponse {
    message: MessageDto,
    inference: CompletedInference,
}

#[derive(Serialize)]
struct CompletedInference {
    status: &'static str,
}

#[derive(Serialize)]
struct CapabilitiesResponse {
    completion: CompletionCapability,
    sync: SyncHealth,
}

#[derive(Serialize)]
struct CompletionCapability {
    configured: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ChatSummary {
    chat_id: String,
    title: String,
}

#[derive(Serialize)]
struct ChatListResponse {
    chats: Vec<ChatSummary>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ChatHistoryResponse {
    chat_id: String,
    title: String,
    messages: Vec<MessageDto>,
    has_more: bool,
    next_cursor: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ChatHistoryQuery {
    cursor: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
struct HistoryPosition {
    author: String,
    seq: u64,
    message_id: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct HistoryCursor {
    version: u8,
    chat_id: String,
    position: HistoryPosition,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MessageDto {
    message_id: String,
    parent_id: Option<String>,
    role: String,
    text: String,
    model: Option<String>,
    author: String,
    seq: u64,
}

impl From<ChatMessage> for MessageDto {
    fn from(message: ChatMessage) -> Self {
        Self {
            message_id: message.message_id,
            parent_id: message.parent_id,
            role: message.role,
            text: message.text,
            model: message.model,
            author: message.author,
            seq: message.seq,
        }
    }
}

#[derive(Serialize)]
struct CreateMessageResponse {
    message: MessageDto,
    inference: InferenceStatus,
}

#[derive(Serialize)]
struct InferenceStatus {
    status: &'static str,
    reason: &'static str,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct CreateChatRequest {
    #[serde(default, deserialize_with = "deserialize_title")]
    title: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RenameChatRequest {
    title: String,
}

fn deserialize_title<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    String::deserialize(deserializer).map(Some)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct CreateMessageRequest {
    text: String,
    #[serde(default)]
    parent_id: Option<String>,
}

#[derive(Serialize)]
struct ApiError {
    error: &'static str,
}

pub fn routes(store: Store) -> Router {
    routes_with_services(store, None, SyncStatus::disabled())
}

pub fn routes_with_services(
    store: Store,
    completer: Option<Arc<dyn ChatCompleter>>,
    sync_status: SyncStatus,
) -> Router {
    let mut cursor_secret = [0; 32];
    SystemRandom::new()
        .fill(&mut cursor_secret)
        .expect("generate chat history cursor session key");
    let history_cursor_key = Arc::new(hmac::Key::new(hmac::HMAC_SHA256, &cursor_secret));

    Router::new()
        .route("/api/chats", get(list_chats).post(create_chat))
        .route("/api/chats/capabilities", get(capabilities))
        .route("/api/chats/{chat_id}/title", post(rename_chat))
        .route("/api/chats/{chat_id}", get(chat_history))
        .route(
            "/api/chats/{chat_id}/messages",
            axum::routing::post(create_message),
        )
        .route(
            "/api/chats/{chat_id}/messages/{message_id}/completion",
            axum::routing::post(complete_message),
        )
        .route(
            "/api/chats/{chat_id}/messages/{message_id}/progress",
            get(message_progress),
        )
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BODY_BYTES))
        .with_state(ChatState {
            store,
            history_cursor_key,
            completer,
            sync_status,
            pending: Arc::new(Mutex::new(HashMap::new())),
            progress: Arc::new(Mutex::new(HashMap::new())),
            failures: Arc::new(Mutex::new(VecDeque::new())),
        })
}

async fn capabilities(State(state): State<ChatState>) -> Response {
    axum::Json(CapabilitiesResponse {
        completion: CompletionCapability {
            configured: state.completer.is_some(),
        },
        sync: state.sync_status.snapshot(),
    })
    .into_response()
}

async fn list_chats(State(state): State<ChatState>) -> Response {
    let chats = state
        .store
        .conversations()
        .into_iter()
        .filter_map(|conversation| {
            conversation.title.map(|title| ChatSummary {
                chat_id: conversation.chat_id,
                title,
            })
        })
        .collect();
    axum::Json(ChatListResponse { chats }).into_response()
}

async fn create_chat(State(state): State<ChatState>, body: Bytes) -> Response {
    let request = match decode_body::<CreateChatRequest>(&body) {
        Ok(request) => request,
        Err(response) => return response,
    };
    let title = request.title.unwrap_or_else(|| "New chat".to_owned());
    if title.len() > MAX_TITLE_BYTES {
        return api_error(StatusCode::BAD_REQUEST, "title_too_long");
    }

    let Some(chat_id) = unique_chat_id(&state.store) else {
        return api_error(StatusCode::INTERNAL_SERVER_ERROR, "storage_unavailable");
    };
    if state
        .store
        .append(ChatEvent::Create {
            chat_id: chat_id.clone(),
            title: title.clone(),
        })
        .is_err()
    {
        return api_error(StatusCode::INTERNAL_SERVER_ERROR, "storage_unavailable");
    }

    (
        StatusCode::CREATED,
        axum::Json(ChatSummary { chat_id, title }),
    )
        .into_response()
}

async fn rename_chat(
    State(state): State<ChatState>,
    Path(chat_id): Path<String>,
    body: Bytes,
) -> Response {
    let request = match decode_body::<RenameChatRequest>(&body) {
        Ok(request) => request,
        Err(response) => return response,
    };
    let title = request.title.trim();
    if title.is_empty() {
        return api_error(StatusCode::BAD_REQUEST, "title_empty");
    }
    if title.len() > MAX_TITLE_BYTES {
        return api_error(StatusCode::BAD_REQUEST, "title_too_long");
    }
    if existing_chat(&state.store, &chat_id).is_none() {
        return api_error(StatusCode::NOT_FOUND, "chat_not_found");
    }
    let title = title.to_owned();
    if state
        .store
        .append(ChatEvent::Title {
            chat_id: chat_id.clone(),
            title: title.clone(),
        })
        .is_err()
    {
        return api_error(StatusCode::INTERNAL_SERVER_ERROR, "storage_unavailable");
    }
    axum::Json(ChatSummary { chat_id, title }).into_response()
}

async fn chat_history(
    State(state): State<ChatState>,
    Path(chat_id): Path<String>,
    Query(query): Query<ChatHistoryQuery>,
) -> Response {
    let Some(mut conversation) = existing_chat(&state.store, &chat_id) else {
        return api_error(StatusCode::NOT_FOUND, "chat_not_found");
    };
    let Some(title) = conversation.title.take() else {
        return api_error(StatusCode::NOT_FOUND, "chat_not_found");
    };
    let cursor_position = match query.cursor.as_deref() {
        Some(cursor) => match decode_history_cursor(&state.history_cursor_key, &chat_id, cursor) {
            Some(cursor) => Some(cursor.position),
            None => return api_error(StatusCode::BAD_REQUEST, "invalid_cursor"),
        },
        None => None,
    };

    conversation.messages.sort_by_key(history_position);
    let end = cursor_position
        .as_ref()
        .map_or(conversation.messages.len(), |cursor| {
            conversation
                .messages
                .partition_point(|message| history_position(message).cmp(cursor).is_lt())
        });
    let start = end.saturating_sub(HISTORY_PAGE_SIZE);
    let has_more = start > 0;
    let page = &conversation.messages[start..end];
    let next_cursor = has_more.then(|| {
        encode_history_cursor(
            &state.history_cursor_key,
            &chat_id,
            history_position(page.first().expect("non-empty history page")),
        )
    });

    axum::Json(ChatHistoryResponse {
        chat_id: conversation.chat_id,
        title,
        messages: page.iter().cloned().map(MessageDto::from).collect(),
        has_more,
        next_cursor,
    })
    .into_response()
}

fn history_position(message: &ChatMessage) -> HistoryPosition {
    HistoryPosition {
        author: message.author.clone(),
        seq: message.seq,
        message_id: message.message_id.clone(),
    }
}

fn encode_history_cursor(key: &hmac::Key, chat_id: &str, position: HistoryPosition) -> String {
    let mut payload = serde_json::to_vec(&HistoryCursor {
        version: 1,
        chat_id: chat_id.to_owned(),
        position,
    })
    .expect("serialize history cursor");
    payload.extend_from_slice(hmac::sign(key, &payload).as_ref());
    URL_SAFE_NO_PAD.encode(payload)
}

fn decode_history_cursor(key: &hmac::Key, chat_id: &str, cursor: &str) -> Option<HistoryCursor> {
    if cursor.is_empty() || cursor.len() > MAX_HISTORY_CURSOR_BYTES {
        return None;
    }
    let bytes = URL_SAFE_NO_PAD.decode(cursor).ok()?;
    if URL_SAFE_NO_PAD.encode(&bytes) != cursor || bytes.len() <= 32 {
        return None;
    }
    let (payload, tag) = bytes.split_at(bytes.len() - 32);
    hmac::verify(key, payload, tag).ok()?;
    let cursor: HistoryCursor = serde_json::from_slice(payload).ok()?;
    (cursor.version == 1
        && cursor.chat_id == chat_id
        && !cursor.position.author.is_empty()
        && cursor.position.seq > 0
        && !cursor.position.message_id.is_empty())
    .then_some(cursor)
}

async fn create_message(
    State(state): State<ChatState>,
    Path(chat_id): Path<String>,
    body: Bytes,
) -> Response {
    let request = match decode_body::<CreateMessageRequest>(&body) {
        Ok(request) => request,
        Err(response) => return response,
    };
    if request.text.len() > MAX_TEXT_BYTES {
        return api_error(StatusCode::BAD_REQUEST, "message_too_long");
    }

    let Some(conversation) = existing_chat(&state.store, &chat_id) else {
        return api_error(StatusCode::NOT_FOUND, "chat_not_found");
    };
    if request.parent_id.as_ref().is_some_and(|parent_id| {
        !conversation
            .messages
            .iter()
            .any(|message| &message.message_id == parent_id)
    }) {
        return api_error(StatusCode::BAD_REQUEST, "invalid_parent");
    }
    let Some(message_id) = unique_message_id(&conversation) else {
        return api_error(StatusCode::INTERNAL_SERVER_ERROR, "storage_unavailable");
    };

    let event = ChatEvent::Message {
        chat_id,
        message_id,
        parent_id: request.parent_id,
        role: "user".to_owned(),
        text: request.text,
        model: None,
    };
    let envelope = match state.store.append(event) {
        Ok(envelope) => envelope,
        Err(_) => return api_error(StatusCode::INTERNAL_SERVER_ERROR, "storage_unavailable"),
    };
    let ChatEvent::Message {
        message_id,
        parent_id,
        role,
        text,
        model,
        ..
    } = envelope.event
    else {
        return api_error(StatusCode::INTERNAL_SERVER_ERROR, "storage_unavailable");
    };

    (
        StatusCode::CREATED,
        axum::Json(CreateMessageResponse {
            message: MessageDto {
                message_id,
                parent_id,
                role,
                text,
                model,
                author: envelope.author,
                seq: envelope.seq,
            },
            inference: InferenceStatus {
                status: "unavailable",
                reason: "not_implemented",
            },
        }),
    )
        .into_response()
}

async fn complete_message(
    State(state): State<ChatState>,
    Path((chat_id, message_id)): Path<(String, String)>,
    body: Bytes,
) -> Response {
    let Some(conversation) = existing_chat(&state.store, &chat_id) else {
        return api_error(StatusCode::NOT_FOUND, "chat_not_found");
    };
    let request = match decode_body::<CompletionRequest>(&body) {
        Ok(request) => request,
        Err(response) => return response,
    };
    if request.model_id.trim().is_empty() || request.model_id.len() > MAX_MODEL_ID_BYTES {
        return api_error(StatusCode::BAD_REQUEST, "invalid_model_id");
    }

    let Some(target) = conversation
        .messages
        .iter()
        .find(|message| message.message_id == message_id)
    else {
        return api_error(StatusCode::BAD_REQUEST, "invalid_target_message");
    };
    if target.role != "user" {
        return api_error(StatusCode::BAD_REQUEST, "invalid_target_message");
    }
    if let Some(message) = existing_completion(&conversation, &message_id, &request.model_id) {
        return completed_response(message);
    }
    if target.author != state.store.identity().node_id {
        return api_error(StatusCode::CONFLICT, "completion_not_locally_owned");
    }
    let Some(messages) = ancestry_to_target(&conversation, &message_id) else {
        return api_error(StatusCode::BAD_REQUEST, "invalid_target_message");
    };
    let Some(completer) = state.completer.clone() else {
        return api_error(StatusCode::BAD_GATEWAY, "completion_failed");
    };

    let key = (chat_id.clone(), message_id.clone());
    let (progress_tx, progress_rx) = watch::channel(String::new());
    {
        let mut pending = agent_lock(&state.pending);
        match pending.get(&key) {
            Some(CompletionState::Pending) => {
                return api_error(StatusCode::CONFLICT, "completion_pending");
            }
            Some(CompletionState::Unknown) => {
                return api_error(StatusCode::BAD_GATEWAY, "completion_outcome_unknown");
            }
            None => match state.store.claim_completion(
                &chat_id,
                &message_id,
                &target.author,
                &request.model_id,
            ) {
                Ok(true) => {
                    pending.insert(key.clone(), CompletionState::Pending);
                    agent_lock(&state.progress).insert(
                        key.clone(),
                        CompletionProgress {
                            model_id: request.model_id.clone(),
                            text: progress_rx,
                        },
                    );
                    agent_lock(&state.failures).retain(|failure| {
                        failure.key != key || failure.model_id != request.model_id
                    });
                }
                Ok(false) => {
                    return api_error(StatusCode::BAD_GATEWAY, "completion_outcome_unknown");
                }
                Err(_) => {
                    return api_error(StatusCode::INTERNAL_SERVER_ERROR, "storage_unavailable");
                }
            },
        }
    }
    let store = state.store.clone();
    let pending = state.pending.clone();
    let progress = state.progress.clone();
    let failures = state.failures.clone();
    let model_id = request.model_id;
    let operation_chat_id = chat_id.clone();
    let operation_message_id = message_id.clone();
    let user_author = target.author.clone();
    let (tx, rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let gate = CompletionGate {
            pending,
            progress,
            failures,
            key,
            completed: false,
        };
        match completer
            .complete_with_progress(model_id.clone(), messages, progress_tx)
            .await
        {
            Err(CompletionError::OutcomeUnknown) => {
                gate.mark_unknown();
                let _ = tx.send(Err(CompletionError::OutcomeUnknown));
            }
            Err(CompletionError::Definitive) => {
                if store
                    .mark_completion_retryable(
                        &operation_chat_id,
                        &operation_message_id,
                        &user_author,
                        &model_id,
                    )
                    .is_ok()
                {
                    gate.fail(model_id.clone());
                    let _ = tx.send(Err(CompletionError::Definitive));
                } else {
                    gate.mark_unknown();
                    let _ = tx.send(Err(CompletionError::OutcomeUnknown));
                }
            }
            Ok(result) if result.model_id != model_id || result.text.len() > MAX_TEXT_BYTES => {
                gate.mark_unknown();
                let _ = tx.send(Err(CompletionError::OutcomeUnknown));
            }
            Ok(result) => {
                match persist_completion(&store, &operation_chat_id, &operation_message_id, result)
                {
                    Ok(message) => {
                        gate.release();
                        let _ = tx.send(Ok(message));
                    }
                    Err(()) => {
                        gate.mark_unknown();
                        let _ = tx.send(Err(CompletionError::OutcomeUnknown));
                    }
                }
            }
        }
    });

    match rx.await {
        Ok(Ok(message)) => completed_response(message),
        Ok(Err(CompletionError::Definitive)) => {
            api_error(StatusCode::BAD_GATEWAY, "completion_failed")
        }
        Ok(Err(CompletionError::OutcomeUnknown)) | Err(_) => {
            api_error(StatusCode::BAD_GATEWAY, "completion_outcome_unknown")
        }
    }
}

async fn message_progress(
    State(state): State<ChatState>,
    Path((chat_id, message_id)): Path<(String, String)>,
    Query(query): Query<ProgressQuery>,
) -> Response {
    if query
        .model_id
        .as_ref()
        .is_some_and(|model_id| model_id.trim().is_empty() || model_id.len() > MAX_MODEL_ID_BYTES)
    {
        return api_error(StatusCode::BAD_REQUEST, "invalid_model_id");
    }

    let Some(conversation) = existing_chat(&state.store, &chat_id) else {
        return progress_response("unknown", String::new(), None);
    };
    if !conversation
        .messages
        .iter()
        .any(|message| message.message_id == message_id && message.role == "user")
    {
        return progress_response("unknown", String::new(), None);
    }
    let key = (chat_id, message_id);

    if let Some(active) = agent_lock(&state.progress).get(&key).cloned()
        && query
            .model_id
            .as_ref()
            .is_none_or(|model_id| model_id == &active.model_id)
    {
        let text = active.text.borrow().clone();
        return progress_response(
            "generating",
            if text.len() <= MAX_CHAT_PROGRESS_BYTES {
                text
            } else {
                String::new()
            },
            Some(active.model_id),
        );
    }

    if let Some(message) = conversation
        .messages
        .iter()
        .filter(|message| {
            message.role == "assistant"
                && message.parent_id.as_deref() == Some(key.1.as_str())
                && query
                    .model_id
                    .as_ref()
                    .is_none_or(|model_id| message.model.as_ref() == Some(model_id))
        })
        .max_by_key(|message| message.seq)
    {
        return progress_response("completed", message.text.clone(), message.model.clone());
    }

    if agent_lock(&state.pending).get(&key) == Some(&CompletionState::Unknown) {
        return progress_response("unknown", String::new(), None);
    }
    if let Some(failure) = agent_lock(&state.failures).iter().rev().find(|failure| {
        failure.key == key
            && query
                .model_id
                .as_ref()
                .is_none_or(|model_id| model_id == &failure.model_id)
    }) {
        return progress_response("failed", String::new(), Some(failure.model_id.clone()));
    }
    progress_response("idle", String::new(), query.model_id)
}

fn progress_response(status: &'static str, text: String, model_id: Option<String>) -> Response {
    axum::Json(MessageProgressResponse {
        status,
        text,
        model_id,
    })
    .into_response()
}

fn persist_completion(
    store: &Store,
    chat_id: &str,
    parent_id: &str,
    result: CompletionResult,
) -> Result<ChatMessage, ()> {
    let conversation = existing_chat(store, chat_id).ok_or(())?;
    let Some(message_id_new) = unique_message_id(&conversation) else {
        return Err(());
    };
    let envelope = store
        .append(ChatEvent::Message {
            chat_id: chat_id.to_owned(),
            message_id: message_id_new,
            parent_id: Some(parent_id.to_owned()),
            role: "assistant".to_owned(),
            text: result.text,
            model: Some(result.model_id),
        })
        .map_err(|_| ())?;
    let ChatEvent::Message {
        message_id,
        parent_id,
        role,
        text,
        model,
        ..
    } = envelope.event
    else {
        return Err(());
    };
    Ok(ChatMessage {
        message_id,
        parent_id,
        role,
        text,
        model,
        author: envelope.author,
        seq: envelope.seq,
    })
}

fn existing_completion(
    conversation: &Conversation,
    parent_id: &str,
    model_id: &str,
) -> Option<ChatMessage> {
    conversation
        .messages
        .iter()
        .find(|message| {
            message.role == "assistant"
                && message.parent_id.as_deref() == Some(parent_id)
                && message.model.as_deref() == Some(model_id)
        })
        .cloned()
}

fn ancestry_to_target(
    conversation: &Conversation,
    target_id: &str,
) -> Option<Vec<CompletionPromptMessage>> {
    let by_id: HashMap<_, _> = conversation
        .messages
        .iter()
        .map(|message| (message.message_id.as_str(), message))
        .collect();
    let mut path = Vec::new();
    let mut next = Some(target_id);
    let mut seen = HashSet::new();
    while let Some(message_id) = next {
        if !seen.insert(message_id) {
            return None;
        }
        let message = by_id.get(message_id)?;
        path.push(CompletionPromptMessage {
            role: message.role.clone(),
            text: message.text.clone(),
        });
        next = message.parent_id.as_deref();
    }
    path.reverse();
    (path.first()?.role == "user" && path.last()?.role == "user").then_some(path)
}

fn completed_response(message: ChatMessage) -> Response {
    axum::Json(CompletionResponse {
        message: MessageDto::from(message),
        inference: CompletedInference {
            status: "completed",
        },
    })
    .into_response()
}

fn decode_body<T: DeserializeOwned>(body: &[u8]) -> Result<T, Response> {
    serde_json::from_slice(body).map_err(|_| api_error(StatusCode::BAD_REQUEST, "invalid_request"))
}

fn existing_chat(store: &Store, chat_id: &str) -> Option<Conversation> {
    store
        .conversation(chat_id)
        .filter(|conversation| conversation.title.is_some())
}

fn unique_chat_id(store: &Store) -> Option<String> {
    (0..ID_ATTEMPTS).find_map(|_| {
        let id = new_id()?;
        store.conversation(&id).is_none().then_some(id)
    })
}

fn unique_message_id(conversation: &Conversation) -> Option<String> {
    (0..ID_ATTEMPTS).find_map(|_| {
        let id = new_id()?;
        conversation
            .messages
            .iter()
            .all(|message| message.message_id != id)
            .then_some(id)
    })
}

fn new_id() -> Option<String> {
    let mut bytes = [0; ID_RANDOM_BYTES];
    SystemRandom::new().fill(&mut bytes).ok()?;
    Some(URL_SAFE_NO_PAD.encode(bytes))
}

fn api_error(status: StatusCode, error: &'static str) -> Response {
    (status, axum::Json(ApiError { error })).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_or_rejected_acp_completion_is_ambiguous() {
        assert!(matches!(
            decode_acp_completion(Ok(serde_json::json!({"modelId": "model-a"}))),
            Err(CompletionError::OutcomeUnknown)
        ));
        assert!(matches!(
            decode_acp_completion(Err(RequestFailure::Rejected("provider error".into()))),
            Err(CompletionError::OutcomeUnknown)
        ));
    }
}
