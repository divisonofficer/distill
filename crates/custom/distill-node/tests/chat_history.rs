use std::path::Path;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use distill_node::chat_store::{ChatEvent, Store};
use distill_node::chats::routes;
use serde_json::{Value, json};
use tower::ServiceExt as _;

fn store_with_chat(path: &Path, chat_id: &str, count: usize) -> Store {
    let store = Store::open(path).unwrap();
    store
        .append(ChatEvent::Create {
            chat_id: chat_id.into(),
            title: chat_id.into(),
        })
        .unwrap();
    let mut parent_id = None;
    for index in 0..count {
        let message_id = format!("message-{index:03}");
        store
            .append(ChatEvent::Message {
                chat_id: chat_id.into(),
                message_id: message_id.clone(),
                parent_id: parent_id.clone(),
                role: if index % 2 == 0 { "user" } else { "assistant" }.into(),
                text: format!("text-{index:03}"),
                model: (index % 2 == 1).then(|| "model-a".into()),
            })
            .unwrap();
        parent_id = Some(message_id);
    }
    store
}

async fn response_json(app: axum::Router, uri: &str) -> (StatusCode, Value) {
    let response = app
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
async fn latest_and_older_pages_have_no_gaps_or_duplicates_and_keep_boundary_parent() {
    let dir = tempfile::tempdir().unwrap();
    let store = store_with_chat(&dir.path().join("store"), "chat", 121);
    let node_id = store.identity().node_id;
    let app = routes(store);

    let (status, latest) = response_json(app.clone(), "/api/chats/chat").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(latest["messages"].as_array().unwrap().len(), 50);
    assert_eq!(latest["messages"][0]["messageId"], "message-071");
    assert_eq!(latest["messages"][49]["messageId"], "message-120");
    assert_eq!(latest["messages"][0]["parentId"], "message-070");
    assert_eq!(latest["hasMore"], true);
    let cursor = latest["nextCursor"].as_str().unwrap();

    let (status, older) =
        response_json(app.clone(), &format!("/api/chats/chat?cursor={cursor}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(older["messages"].as_array().unwrap().len(), 50);
    assert_eq!(older["messages"][0]["messageId"], "message-021");
    assert_eq!(older["messages"][49]["messageId"], "message-070");
    assert_eq!(older["hasMore"], true);
    let cursor = older["nextCursor"].as_str().unwrap();

    let (status, oldest) = response_json(app, &format!("/api/chats/chat?cursor={cursor}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(oldest["messages"].as_array().unwrap().len(), 21);
    assert_eq!(oldest["messages"][0]["messageId"], "message-000");
    assert_eq!(oldest["messages"][20]["messageId"], "message-020");
    assert_eq!(oldest["hasMore"], false);
    assert_eq!(oldest["nextCursor"], Value::Null);

    let combined = oldest["messages"]
        .as_array()
        .unwrap()
        .iter()
        .chain(older["messages"].as_array().unwrap())
        .chain(latest["messages"].as_array().unwrap())
        .map(|message| message["messageId"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(combined.len(), 121);
    assert_eq!(
        combined
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len(),
        121
    );
    assert_eq!(combined[0], "message-000");
    assert_eq!(combined[120], "message-120");
    assert_eq!(latest["messages"][0]["author"], node_id);
    assert_eq!(latest["messages"][2]["model"], "model-a");
}

#[tokio::test]
async fn rejects_invalid_cross_chat_and_cross_session_cursors() {
    let dir = tempfile::tempdir().unwrap();
    let store = store_with_chat(&dir.path().join("first"), "first", 51);
    store
        .append(ChatEvent::Create {
            chat_id: "second".into(),
            title: "second".into(),
        })
        .unwrap();
    let app = routes(store.clone());

    let (_, page) = response_json(app.clone(), "/api/chats/first").await;
    let cursor = page["nextCursor"].as_str().unwrap();
    let (status, invalid) = response_json(app.clone(), "/api/chats/first?cursor=invalid").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(invalid["error"], "invalid_cursor");
    let (status, cross_chat) =
        response_json(app, &format!("/api/chats/second?cursor={cursor}")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(cross_chat["error"], "invalid_cursor");

    let other_session = routes(store.clone());
    let (status, cross_session) =
        response_json(other_session, &format!("/api/chats/first?cursor={cursor}")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(cross_session["error"], "invalid_cursor");
    assert!(store.conversation("second").unwrap().messages.is_empty());
}

#[tokio::test]
async fn empty_history_returns_no_older_cursor() {
    let dir = tempfile::tempdir().unwrap();
    let store = store_with_chat(&dir.path().join("store"), "empty", 0);
    let (status, history) = response_json(routes(store), "/api/chats/empty").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(history["messages"], json!([]));
    assert_eq!(history["hasMore"], false);
    assert_eq!(history["nextCursor"], Value::Null);
}
