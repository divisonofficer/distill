use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use distill_node::agent::AgentConfig;
use distill_node::chat_store::{ChatEvent, Store};
use distill_node::chat_sync::SyncStatus;
use distill_node::chats::{
    AcpChatCompleter, ChatCompleter, CompletionError, CompletionPromptMessage, CompletionResult,
    routes_with_services,
};
use distill_node::keeper::Keeper;
use serde_json::{Value, json};
use tokio::sync::{Notify, watch};
use tower::ServiceExt as _;

struct MockCompleter {
    calls: AtomicUsize,
    prompts: Mutex<Vec<Vec<CompletionPromptMessage>>>,
    fail: bool,
    unknown: bool,
    started: Option<Arc<Notify>>,
    release: Option<Arc<Notify>>,
}

impl MockCompleter {
    fn new(fail: bool) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            prompts: Mutex::new(Vec::new()),
            fail,
            unknown: false,
            started: None,
            release: None,
        }
    }

    fn ambiguous() -> Self {
        Self {
            calls: AtomicUsize::new(0),
            prompts: Mutex::new(Vec::new()),
            fail: false,
            unknown: true,
            started: None,
            release: None,
        }
    }

    fn gated(started: Arc<Notify>, release: Arc<Notify>) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            prompts: Mutex::new(Vec::new()),
            fail: false,
            unknown: false,
            started: Some(started),
            release: Some(release),
        }
    }
}

impl ChatCompleter for MockCompleter {
    fn complete(
        &self,
        model_id: String,
        messages: Vec<CompletionPromptMessage>,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<CompletionResult, CompletionError>> + Send>,
    > {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.prompts.lock().unwrap().push(messages);
        let fail = self.fail;
        let unknown = self.unknown;
        let started = self.started.clone();
        let release = self.release.clone();
        Box::pin(async move {
            if let Some(started) = started {
                started.notify_one();
            }
            if let Some(release) = release {
                release.notified().await;
            }
            if unknown {
                Err(CompletionError::OutcomeUnknown)
            } else if fail {
                Err(CompletionError::Definitive)
            } else {
                Ok(CompletionResult {
                    model_id,
                    text: "mock answer".to_owned(),
                })
            }
        })
    }

    fn complete_with_progress(
        &self,
        model_id: String,
        messages: Vec<CompletionPromptMessage>,
        progress: watch::Sender<String>,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<CompletionResult, CompletionError>> + Send>,
    > {
        progress.send_replace("partial snapshot".to_owned());
        self.complete(model_id, messages)
    }
}

fn fixture() -> (tempfile::TempDir, Store, String) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("store")).unwrap();
    store
        .append(ChatEvent::Create {
            chat_id: "chat".into(),
            title: "Test".into(),
        })
        .unwrap();
    store
        .append(ChatEvent::Message {
            chat_id: "chat".into(),
            message_id: "root".into(),
            parent_id: None,
            role: "user".into(),
            text: "first question".into(),
            model: None,
            text_files: Vec::new(),
        })
        .unwrap();
    store
        .append(ChatEvent::Message {
            chat_id: "chat".into(),
            message_id: "middle".into(),
            parent_id: Some("root".into()),
            role: "assistant".into(),
            text: "prior answer".into(),
            model: Some("old-model".into()),
            text_files: Vec::new(),
        })
        .unwrap();
    store
        .append(ChatEvent::Message {
            chat_id: "chat".into(),
            message_id: "target".into(),
            parent_id: Some("middle".into()),
            role: "user".into(),
            text: "second question".into(),
            model: None,
            text_files: Vec::new(),
        })
        .unwrap();
    store
        .append(ChatEvent::Message {
            chat_id: "chat".into(),
            message_id: "sibling".into(),
            parent_id: Some("root".into()),
            role: "user".into(),
            text: "other branch".into(),
            model: None,
            text_files: Vec::new(),
        })
        .unwrap();
    (dir, store, "target".into())
}

fn request(message_id: &str, model_id: &str) -> Request<Body> {
    completion_request("chat", message_id, model_id)
}

fn completion_request(chat_id: &str, message_id: &str, model_id: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(format!(
            "/api/chats/{chat_id}/messages/{message_id}/completion"
        ))
        .header("content-type", "application/json")
        .body(Body::from(json!({"modelId": model_id}).to_string()))
        .unwrap()
}

fn progress_request(chat_id: &str, message_id: &str, model_id: Option<&str>) -> Request<Body> {
    let query = model_id.map_or(String::new(), |model_id| format!("?modelId={model_id}"));
    Request::builder()
        .method("GET")
        .uri(format!(
            "/api/chats/{chat_id}/messages/{message_id}/progress{query}"
        ))
        .body(Body::empty())
        .unwrap()
}

async fn response_json(router: axum::Router, req: Request<Body>) -> (StatusCode, Value) {
    let response = router.oneshot(req).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

fn router(store: Store, completer: Arc<dyn ChatCompleter>) -> axum::Router {
    routes_with_services(store, Some(completer), SyncStatus::disabled())
}

#[tokio::test]
async fn completion_uses_only_ancestry_persists_and_retry_reuses_child() {
    let (dir, store, target) = fixture();
    let mock = Arc::new(MockCompleter::new(false));
    let app = router(store.clone(), mock.clone());
    let (status, body) = response_json(app.clone(), request(&target, "model-a")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["inference"]["status"], "completed");
    assert_eq!(body["message"]["text"], "mock answer");
    {
        let prompts = mock.prompts.lock().unwrap();
        assert_eq!(
            prompts[0]
                .iter()
                .map(|message| (message.role.as_str(), message.text.as_str()))
                .collect::<Vec<_>>(),
            [
                ("user", "first question"),
                ("assistant", "prior answer"),
                ("user", "second question"),
            ]
        );
    }

    let (status, retry) = response_json(app, request(&target, "model-a")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(retry["message"]["messageId"], body["message"]["messageId"]);
    assert_eq!(mock.calls.load(Ordering::SeqCst), 1);

    let reopened = Store::open(&dir.path().join("store")).unwrap();
    let conversation = reopened.conversation("chat").unwrap();
    assert!(conversation.messages.iter().any(|message| {
        message.message_id == body["message"]["messageId"]
            && message.role == "assistant"
            && message.model.as_deref() == Some("model-a")
    }));
}

#[tokio::test]
async fn progress_get_tracks_only_its_chat_and_resolves_saved_answer_without_generating() {
    let (_dir, store, target) = fixture();
    store
        .append(ChatEvent::Create {
            chat_id: "other-chat".into(),
            title: "Other".into(),
        })
        .unwrap();
    store
        .append(ChatEvent::Message {
            chat_id: "other-chat".into(),
            message_id: "other-target".into(),
            parent_id: None,
            role: "user".into(),
            text: "other question".into(),
            model: None,
            text_files: Vec::new(),
        })
        .unwrap();

    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let mock = Arc::new(MockCompleter::gated(started.clone(), release.clone()));
    let app = router(store, mock.clone());
    let first_app = app.clone();
    let first =
        tokio::spawn(async move { response_json(first_app, request(&target, "model-a")).await });
    tokio::time::timeout(Duration::from_secs(2), started.notified())
        .await
        .unwrap();

    let (status, active) =
        response_json(app.clone(), progress_request("chat", "target", None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(active["status"], "generating");
    assert_eq!(active["text"], "partial snapshot");
    assert_eq!(active["modelId"], "model-a");

    let (status, isolated) = response_json(
        app.clone(),
        progress_request("other-chat", "other-target", None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(isolated["status"], "idle");
    assert_eq!(isolated["text"], "");
    assert_eq!(mock.calls.load(Ordering::SeqCst), 1);

    release.notify_one();
    let (status, final_response) = first.await.unwrap();
    assert_eq!(status, StatusCode::OK);
    assert_eq!(final_response["message"]["text"], "mock answer");
    let (status, completed) =
        response_json(app, progress_request("chat", "target", Some("model-a"))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(completed["status"], "completed");
    assert_eq!(completed["text"], "mock answer");
    assert_eq!(completed["modelId"], "model-a");
    assert_eq!(mock.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn pending_completion_claim_stays_blocked_after_restart() {
    let (dir, store, target) = fixture();
    let user_author = store
        .conversation("chat")
        .unwrap()
        .messages
        .into_iter()
        .find(|message| message.message_id == target)
        .unwrap()
        .author;
    assert!(
        store
            .claim_completion("chat", &target, &user_author, "model-a")
            .unwrap()
    );
    drop(store);

    let reopened = Store::open(&dir.path().join("store")).unwrap();
    let mock = Arc::new(MockCompleter::new(false));
    let (status, body) =
        response_json(router(reopened, mock.clone()), request(&target, "model-a")).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(body["error"], "completion_outcome_unknown");
    assert_eq!(mock.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn imported_user_message_cannot_authorize_local_completion() {
    let source_dir = tempfile::tempdir().unwrap();
    let source = Store::open(&source_dir.path().join("store")).unwrap();
    source
        .append(ChatEvent::Create {
            chat_id: "shared-chat".into(),
            title: "Shared".into(),
        })
        .unwrap();
    source
        .append(ChatEvent::Message {
            chat_id: "shared-chat".into(),
            message_id: "shared-user".into(),
            parent_id: None,
            role: "user".into(),
            text: "remote question".into(),
            model: None,
            text_files: Vec::new(),
        })
        .unwrap();
    let identity = source.identity();

    let local_dir = tempfile::tempdir().unwrap();
    let local = Store::open(&local_dir.path().join("store")).unwrap();
    local
        .import(
            source.export(),
            &BTreeMap::from([(identity.node_id, identity.public_key)]),
        )
        .unwrap();
    let mock = Arc::new(MockCompleter::new(false));
    let (status, body) = response_json(
        router(local, mock.clone()),
        completion_request("shared-chat", "shared-user", "model-a"),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"], "completion_not_locally_owned");
    assert_eq!(mock.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn ambiguous_completion_claim_stays_blocked_after_restart() {
    let (dir, store, target) = fixture();
    let mock = Arc::new(MockCompleter::ambiguous());
    let app = router(store.clone(), mock.clone());
    let (status, body) = response_json(app, request(&target, "model-a")).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(body["error"], "completion_outcome_unknown");
    drop(store);

    let reopened = Store::open(&dir.path().join("store")).unwrap();
    let retry_mock = Arc::new(MockCompleter::new(false));
    let (status, body) = response_json(
        router(reopened, retry_mock.clone()),
        request(&target, "model-a"),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(body["error"], "completion_outcome_unknown");
    assert_eq!(retry_mock.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn proven_pre_dispatch_failure_is_retryable_after_restart() {
    let (dir, store, target) = fixture();
    let failed = Arc::new(MockCompleter::new(true));
    let (status, body) = response_json(
        router(store.clone(), failed.clone()),
        request(&target, "model-a"),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(body["error"], "completion_failed");
    assert_eq!(failed.calls.load(Ordering::SeqCst), 1);
    drop(store);

    let reopened = Store::open(&dir.path().join("store")).unwrap();
    let retry = Arc::new(MockCompleter::new(false));
    let (status, body) =
        response_json(router(reopened, retry.clone()), request(&target, "model-a")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["inference"]["status"], "completed");
    assert_eq!(retry.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn completion_failures_keep_user_message_and_invalid_targets_are_rejected() {
    let (_dir, store, target) = fixture();
    let mock = Arc::new(MockCompleter::new(true));
    let app = router(store.clone(), mock.clone());
    let (status, _) = response_json(app.clone(), request(&target, "model-a")).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(mock.calls.load(Ordering::SeqCst), 1);
    let (retry_status, retry_body) = response_json(app.clone(), request(&target, "model-a")).await;
    assert_eq!(retry_status, StatusCode::BAD_GATEWAY);
    assert_eq!(retry_body["error"], "completion_failed");
    assert_eq!(mock.calls.load(Ordering::SeqCst), 2);
    let conversation = store.conversation("chat").unwrap();
    assert!(
        conversation
            .messages
            .iter()
            .any(|message| message.message_id == target)
    );
    assert!(
        !conversation
            .messages
            .iter()
            .any(|message| message.role == "assistant"
                && message.model.as_deref() == Some("model-a"))
    );
    let (progress_status, progress) = response_json(
        app.clone(),
        progress_request("chat", "target", Some("model-a")),
    )
    .await;
    assert_eq!(progress_status, StatusCode::OK);
    assert_eq!(progress["status"], "failed");
    assert_eq!(progress["text"], "");
    assert_eq!(progress["modelId"], "model-a");

    let (status, _) = response_json(app.clone(), request("middle", "model-a")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = response_json(app.clone(), request("missing", "model-a")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = response_json(app.clone(), request("target", "  ")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let unknown = Request::builder()
        .method("POST")
        .uri("/api/chats/unknown/messages/target/completion")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"modelId":"model-a"}"#))
        .unwrap();
    let (status, _) = response_json(app.clone(), unknown).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let capabilities = Request::builder()
        .method("GET")
        .uri("/api/chats/capabilities")
        .body(Body::empty())
        .unwrap();
    let (status, capabilities) = response_json(app, capabilities).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(capabilities["completion"]["configured"], true);
    assert_eq!(capabilities["sync"]["available"], false);
}

#[tokio::test]
async fn unknown_completion_outcome_blocks_retry_for_process_lifetime() {
    let (_dir, store, target) = fixture();
    let mock = Arc::new(MockCompleter::ambiguous());
    let app = router(store.clone(), mock.clone());
    let (status, body) = response_json(app.clone(), request(&target, "model-a")).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(body["error"], "completion_outcome_unknown");
    let (status, retry) = response_json(app, request(&target, "model-a")).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(retry["error"], "completion_outcome_unknown");
    assert_eq!(mock.calls.load(Ordering::SeqCst), 1);
    assert!(
        store
            .conversation("chat")
            .unwrap()
            .messages
            .iter()
            .all(|message| message.model.as_deref() != Some("model-a"))
    );
}

#[tokio::test]
async fn concurrent_completion_across_routers_claims_once() {
    let (_dir, store, _target) = fixture();
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let mock = Arc::new(MockCompleter::gated(started.clone(), release.clone()));
    let first_app = router(store.clone(), mock.clone());
    let second_app = router(store, mock.clone());
    let first =
        tokio::spawn(async move { response_json(first_app, request("target", "model-a")).await });
    tokio::time::timeout(Duration::from_secs(2), started.notified())
        .await
        .unwrap();

    let (status, body) = response_json(second_app, request("target", "model-a")).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(body["error"], "completion_outcome_unknown");
    assert_eq!(mock.calls.load(Ordering::SeqCst), 1);
    release.notify_one();
    let (status, _) = first.await.unwrap();
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn aborted_http_waiter_keeps_completion_task_and_retry_reuses_result() {
    let (_dir, store, target) = fixture();
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let mock = Arc::new(MockCompleter::gated(started.clone(), release.clone()));
    let app = router(store.clone(), mock.clone());
    let first_app = app.clone();
    let first = tokio::spawn(async move { first_app.oneshot(request(&target, "model-a")).await });
    tokio::time::timeout(Duration::from_secs(2), started.notified())
        .await
        .unwrap();
    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());

    release.notify_one();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if store
                .conversation("chat")
                .unwrap()
                .messages
                .iter()
                .any(|message| {
                    message.role == "assistant" && message.model.as_deref() == Some("model-a")
                })
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let (status, body) = response_json(app, request("target", "model-a")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["message"]["text"], "mock answer");
    assert_eq!(mock.calls.load(Ordering::SeqCst), 1);
}

const MOCK_ACP: &str = r#"
import json, os, sys, time
counter = os.environ["MOCK_ACP_COUNTER"]
calls = os.environ["MOCK_ACP_CALLS"]
try:
    generation = int(open(counter).read())
except (OSError, ValueError):
    generation = 0
open(counter, "w").write(str(generation + 1))
for line in sys.stdin:
    msg = json.loads(line)
    method = msg.get("method")
    if method == "test/exit":
        os._exit(0)
    if method is None or "id" not in msg:
        continue
    params = msg.get("params", {})
    with open(calls, "a") as log:
        log.write(method + "\n")
        log.flush()
    if method == "initialize":
        result = {"authMethods": []}
    elif method == "_x.ai/sessions/list":
        result = {"sessions": []}
    elif method == "_x.ai/chat/complete":
        request_id = params.get("requestId")
        if request_id:
            sys.stdout.write(json.dumps({"jsonrpc":"2.0", "method":"_x.ai/chat/progress", "params":{"requestId":"mismatch", "text":"wrong request"}}) + "\n")
            sys.stdout.write(json.dumps({"jsonrpc":"2.0", "method":"_x.ai/chat/progress", "params":{"requestId":request_id, "text":"partial snapshot"}}) + "\n")
            sys.stdout.flush()
            time.sleep(0.05)
            sys.stdout.write(json.dumps({"jsonrpc":"2.0", "method":"x.ai/chat/progress", "params":{"requestId":request_id, "text":"partial snapshot updated"}}) + "\n")
            sys.stdout.flush()
        time.sleep(float(os.environ.get("MOCK_ACP_DELAY", "0")))
        result = {"modelId": params["modelId"], "text": "real fake ACP answer"}
    else:
        result = {}
    sys.stdout.write(json.dumps({"jsonrpc":"2.0", "id":msg["id"], "result":result}) + "\n")
    sys.stdout.flush()
"#;

fn mock_acp_config(
    dir: &std::path::Path,
    delay: &str,
) -> (AgentConfig, std::path::PathBuf, std::path::PathBuf) {
    let script = dir.join("mock_acp.py");
    let counter = dir.join("counter");
    let calls = dir.join("calls");
    std::fs::write(&script, MOCK_ACP).unwrap();
    (
        AgentConfig {
            distill_bin: "python3".into(),
            args: vec![script.into_os_string()],
            extra_env: vec![
                ("MOCK_ACP_COUNTER".into(), counter.clone().into_os_string()),
                ("MOCK_ACP_CALLS".into(), calls.clone().into_os_string()),
                ("MOCK_ACP_DELAY".into(), delay.into()),
            ],
        },
        counter,
        calls,
    )
}

#[tokio::test]
async fn acp_progress_is_correlated_and_received_before_final_response() {
    let dir = tempfile::tempdir().unwrap();
    let (config, _, _) = mock_acp_config(dir.path(), "0.25");
    let keeper = Keeper::start(config).await.unwrap();
    let completer = AcpChatCompleter::new(keeper);
    let (progress_tx, mut progress_rx) = watch::channel(String::new());
    let completion = tokio::spawn(async move {
        completer
            .complete_with_progress(
                "model-progress".into(),
                vec![CompletionPromptMessage {
                    role: "user".into(),
                    text: "hello".into(),
                }],
                progress_tx,
            )
            .await
    });

    tokio::time::timeout(Duration::from_secs(2), progress_rx.changed())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&*progress_rx.borrow(), "partial snapshot");
    assert!(!completion.is_finished());
    tokio::time::timeout(Duration::from_secs(2), progress_rx.changed())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&*progress_rx.borrow(), "partial snapshot updated");
    assert!(!completion.is_finished());

    let result = completion.await.unwrap().unwrap();
    assert_eq!(result.model_id, "model-progress");
    assert_eq!(result.text, "real fake ACP answer");
}

#[tokio::test]
async fn acp_transport_decoding_disconnect_and_retry_are_durable() {
    let (dir, store, target) = fixture();
    let (config, _counter, calls) = mock_acp_config(dir.path(), "0.15");
    let keeper = Keeper::start(config).await.unwrap();
    let completer = Arc::new(AcpChatCompleter::new(keeper));
    let app = router(store.clone(), completer);
    let first_app = app.clone();
    let first = tokio::spawn(async move { first_app.oneshot(request(&target, "model-acp")).await });
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if std::fs::read_to_string(&calls)
                .unwrap_or_default()
                .contains("_x.ai/chat/complete")
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if store
                .conversation("chat")
                .unwrap()
                .messages
                .iter()
                .any(|message| {
                    message.role == "assistant" && message.text == "real fake ACP answer"
                })
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let (status, body) = response_json(app, request("target", "model-acp")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["message"]["text"], "real fake ACP answer");
    assert_eq!(
        std::fs::read_to_string(calls)
            .unwrap()
            .matches("_x.ai/chat/complete")
            .count(),
        1
    );
}

#[tokio::test]
async fn acp_completer_uses_keeper_recovered_child() {
    let dir = tempfile::tempdir().unwrap();
    let (config, counter, calls) = mock_acp_config(dir.path(), "0");
    let keeper = Keeper::start(config).await.unwrap();
    keeper.handle().notify("test/exit", json!({})).unwrap();
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            let ready = std::fs::read_to_string(&counter)
                .ok()
                .and_then(|generation| generation.parse::<usize>().ok())
                .is_some_and(|generation| generation >= 2)
                && keeper
                    .handle()
                    .request("_x.ai/sessions/list", json!({}))
                    .await
                    .is_ok();
            if ready {
                break;
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    })
    .await
    .expect("keeper reconnects to child");
    let result = AcpChatCompleter::new(keeper)
        .complete(
            "model-recovered".into(),
            vec![CompletionPromptMessage {
                role: "user".into(),
                text: "hello".into(),
            }],
        )
        .await
        .unwrap();
    assert_eq!(result.model_id, "model-recovered");
    assert_eq!(result.text, "real fake ACP answer");
    assert_eq!(
        std::fs::read_to_string(calls)
            .unwrap()
            .matches("_x.ai/chat/complete")
            .count(),
        1
    );
}
