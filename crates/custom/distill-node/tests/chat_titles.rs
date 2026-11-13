use std::path::Path;

use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, StatusCode};
use distill_node::chat_store::{ChatEvent, Store};
use distill_node::chats::routes;
use serde_json::{Value, json};
use tower::ServiceExt as _;

fn store_with_chat(path: &Path, chat_id: &str, title: &str, text: &str) -> Store {
    let store = Store::open(path).unwrap();
    store
        .append(ChatEvent::Create {
            chat_id: chat_id.into(),
            title: title.into(),
        })
        .unwrap();
    store
        .append(ChatEvent::Message {
            chat_id: chat_id.into(),
            message_id: format!("{chat_id}-message"),
            parent_id: None,
            role: "user".into(),
            text: text.into(),
            model: None,
            text_files: Vec::new(),
        })
        .unwrap();
    store
}

async fn request_json(
    app: axum::Router,
    method: Method,
    uri: &str,
    body: Value,
) -> (StatusCode, Value) {
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

#[tokio::test]
async fn rename_trims_unicode_persists_and_updates_list_and_history() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store");
    let store = store_with_chat(&path, "renamed", "New chat", "hello");
    let app = routes(store.clone());
    let (status, response) = request_json(
        app,
        Method::POST,
        "/api/chats/renamed/title",
        json!({"title": "  새 제목 🌱  "}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(response["title"], "새 제목 🌱");
    drop(store);

    let reopened = Store::open(&path).unwrap();
    let app = routes(reopened);
    let (status, list) = request_json(app.clone(), Method::GET, "/api/chats", json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list["chats"][0]["title"], "새 제목 🌱");
    let response = app
        .oneshot(
            Request::get("/api/chats/renamed")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let history: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(history["title"], "새 제목 🌱");
}

#[tokio::test]
async fn rejects_empty_oversized_and_unknown_titles() {
    let dir = tempfile::tempdir().unwrap();
    let store = store_with_chat(&dir.path().join("store"), "chat", "New chat", "hello");
    let app = routes(store);
    let (status, _) = request_json(
        app.clone(),
        Method::POST,
        "/api/chats/chat/title",
        json!({"title": " \n\t "}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = request_json(
        app.clone(),
        Method::POST,
        "/api/chats/chat/title",
        json!({"title": "🌱".repeat(76)}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = request_json(
        app,
        Method::POST,
        "/api/chats/missing/title",
        json!({"title": "Unknown"}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn derives_historical_titles_and_preserves_custom_titles() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store");
    let store = store_with_chat(
        &path,
        "historical",
        "New chat",
        "  첫 메시지\n   제목   이어짐  ",
    );
    store
        .append(ChatEvent::Create {
            chat_id: "custom".into(),
            title: "Custom title".into(),
        })
        .unwrap();
    store
        .append(ChatEvent::Message {
            chat_id: "custom".into(),
            message_id: "custom-message".into(),
            parent_id: None,
            role: "user".into(),
            text: "Do not replace this".into(),
            model: None,
            text_files: Vec::new(),
        })
        .unwrap();
    let store = Store::open(&path).unwrap();
    let conversations = store.conversations();
    assert_eq!(
        conversations
            .iter()
            .find(|conversation| conversation.chat_id == "historical")
            .unwrap()
            .title
            .as_deref(),
        Some("첫 메시지 제목 이어짐")
    );
    assert_eq!(
        conversations
            .iter()
            .find(|conversation| conversation.chat_id == "custom")
            .unwrap()
            .title
            .as_deref(),
        Some("Custom title")
    );
    let app = routes(store);
    let (status, list) = request_json(app.clone(), Method::GET, "/api/chats", json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        list["chats"]
            .as_array()
            .unwrap()
            .iter()
            .find(|chat| chat["chatId"] == "historical")
            .unwrap()["title"],
        "첫 메시지 제목 이어짐"
    );
    let response = app
        .oneshot(
            Request::get("/api/chats/historical")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let history: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(history["title"], "첫 메시지 제목 이어짐");
}
