use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use distill_node::chat_store::{ChatEvent, Envelope, Store};
use distill_node::chat_sync::SyncStatus;
use distill_node::chats::{
    ChatCompleter, CompletionError, CompletionPromptMessage, CompletionResult, routes_with_services,
};
use distill_node::sep::TextFileIn;
use serde_json::{Value, json};
use tower::ServiceExt as _;

#[derive(Default)]
struct Mock {
    calls: AtomicUsize,
    messages: Mutex<Vec<Vec<CompletionPromptMessage>>>,
}
impl ChatCompleter for Mock {
    fn complete(
        &self,
        model_id: String,
        messages: Vec<CompletionPromptMessage>,
    ) -> Pin<Box<dyn Future<Output = Result<CompletionResult, CompletionError>> + Send>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.messages.lock().unwrap().push(messages);
        Box::pin(async move {
            Ok(CompletionResult {
                model_id,
                text: "answer".into(),
            })
        })
    }
}
fn app(store: Store, mock: Arc<Mock>) -> axum::Router {
    routes_with_services(store, Some(mock), SyncStatus::disabled())
}
async fn call(app: axum::Router, method: &str, uri: &str, body: Value) -> (StatusCode, Value) {
    let response = app
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}
fn setup() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("store")).unwrap();
    store
        .append(ChatEvent::Create {
            chat_id: "chat".into(),
            title: "Chat".into(),
        })
        .unwrap();
    (dir, store)
}
fn file(name: &str, text: &str) -> Value {
    json!({"name":name,"text":text})
}
fn message(text: &str, files: Vec<Value>, parent: Option<&str>) -> Value {
    json!({"text":text,"textFiles":files,"parentId":parent})
}

#[tokio::test]
async fn saves_normalizes_replays_reopens_and_allows_attachment_only() {
    let (dir, store) = setup();
    let (status, created) = call(
        routes_with_services(store.clone(), None, SyncStatus::disabled()),
        "POST",
        "/api/chats/chat/messages",
        message("", vec![file("/tmp/note.md", "body")], None),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(created["message"]["text"], "");
    assert_eq!(created["message"]["textFiles"][0]["name"], "note.md");
    let (status, history) = call(
        routes_with_services(store, None, SyncStatus::disabled()),
        "GET",
        "/api/chats/chat",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(history["messages"][0]["textFiles"][0]["text"], "body");
    let reopened = Store::open(&dir.path().join("store")).unwrap();
    assert_eq!(
        reopened.conversation("chat").unwrap().messages[0].text_files[0].name,
        "note.md"
    );
}

#[tokio::test]
async fn invalid_files_do_not_append_event() {
    let (_dir, store) = setup();
    let router = routes_with_services(store.clone(), None, SyncStatus::disabled());
    let invalid = [
        vec![file("x.png", "x")],
        (0..11).map(|i| file(&format!("{i}.txt"), "x")).collect(),
        vec![file("a.txt", &"x".repeat(1024 * 1024)), file("b.txt", "x")],
        vec![json!({"name":"../","text":"x"})],
    ];
    for files in invalid {
        let count = store.export().len();
        let (status, _) = call(
            router.clone(),
            "POST",
            "/api/chats/chat/messages",
            message("", files, None),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(store.export().len(), count);
    }
}

#[tokio::test]
async fn completion_includes_only_ancestral_file_content_and_rejects_oversized_prompt() {
    let (_dir, store) = setup();
    let router = routes_with_services(store.clone(), None, SyncStatus::disabled());
    let (_, root) = call(
        router.clone(),
        "POST",
        "/api/chats/chat/messages",
        message("root", vec![file("root.txt", "ROOT-CONTENT")], None),
    )
    .await;
    let root_id = root["message"]["messageId"].as_str().unwrap();
    let (_, _sibling) = call(
        router.clone(),
        "POST",
        "/api/chats/chat/messages",
        message("sibling", vec![file("other.txt", "OTHER-BRANCH")], None),
    )
    .await;
    let (_, target) = call(
        router.clone(),
        "POST",
        "/api/chats/chat/messages",
        message("target", vec![], Some(root_id)),
    )
    .await;
    let target_id = target["message"]["messageId"].as_str().unwrap();
    let mock = Arc::new(Mock::default());
    let (status, _) = call(
        app(store.clone(), mock.clone()),
        "POST",
        &format!("/api/chats/chat/messages/{target_id}/completion"),
        json!({"modelId":"m"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let prompt = mock.messages.lock().unwrap()[0]
        .iter()
        .map(|m| m.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(prompt.contains("ROOT-CONTENT") && prompt.contains("root.txt"));
    assert!(!prompt.contains("OTHER-BRANCH") && !prompt.contains("other.txt"));

    let (_, huge_root) = call(
        routes_with_services(store.clone(), None, SyncStatus::disabled()),
        "POST",
        "/api/chats/chat/messages",
        message("", vec![file("large.txt", &"x".repeat(1024 * 1024))], None),
    )
    .await;
    let huge_root_id = huge_root["message"]["messageId"].as_str().unwrap();
    let (_, huge_target) = call(
        routes_with_services(store.clone(), None, SyncStatus::disabled()),
        "POST",
        "/api/chats/chat/messages",
        message("next", vec![], Some(huge_root_id)),
    )
    .await;
    let huge_id = huge_target["message"]["messageId"].as_str().unwrap();
    let mock = Arc::new(Mock::default());
    let (status, _) = call(
        app(store, mock.clone()),
        "POST",
        &format!("/api/chats/chat/messages/{huge_id}/completion"),
        json!({"modelId":"m"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(mock.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn signed_event_compatibility_import_and_tamper_rejection() {
    let (dir, source) = setup();
    let empty = source
        .append(ChatEvent::Message {
            chat_id: "chat".into(),
            message_id: "empty".into(),
            parent_id: None,
            role: "user".into(),
            text: String::new(),
            model: None,
            text_files: vec![],
        })
        .unwrap();
    assert!(
        serde_json::to_value(&empty).unwrap()["event"]
            .get("text_files")
            .is_none()
    );
    assert_eq!(
        serde_json::to_string(&empty.event).unwrap(),
        r#"{"type":"message","chat_id":"chat","message_id":"empty","parent_id":null,"role":"user","text":"","model":null}"#
    );
    let legacy: Envelope = serde_json::from_value(serde_json::to_value(&empty).unwrap()).unwrap();
    let create = source
        .export()
        .into_iter()
        .find(|event| matches!(event.event, ChatEvent::Create { .. }))
        .unwrap();
    let file_event = source
        .append(ChatEvent::Message {
            chat_id: "chat".into(),
            message_id: "file".into(),
            parent_id: Some("empty".into()),
            role: "user".into(),
            text: String::new(),
            model: None,
            text_files: vec![TextFileIn {
                name: "note.txt".into(),
                text: "signed".into(),
            }],
        })
        .unwrap();
    let identity = source.identity();
    let target_dir = tempfile::tempdir().unwrap();
    let target = Store::open(&target_dir.path().join("store")).unwrap();
    let trust = BTreeMap::from([(identity.node_id, identity.public_key)]);
    target
        .import(
            vec![create.clone(), legacy.clone(), file_event.clone()],
            &trust,
        )
        .unwrap();
    assert_eq!(
        Store::open(&target_dir.path().join("store"))
            .unwrap()
            .conversation("chat")
            .unwrap()
            .messages
            .last()
            .unwrap()
            .text_files[0]
            .text,
        "signed"
    );
    let mut tampered = file_event;
    if let ChatEvent::Message { text_files, .. } = &mut tampered.event {
        text_files[0].text = "changed".into();
    }
    let other_dir = tempfile::tempdir().unwrap();
    let other = Store::open(&other_dir.path().join("store")).unwrap();
    assert!(
        other
            .import(vec![create, legacy, tampered], &trust)
            .is_err()
    );
    drop(source);
    assert_eq!(
        Store::open(&dir.path().join("store"))
            .unwrap()
            .conversation("chat")
            .unwrap()
            .messages
            .len(),
        2
    );
}
