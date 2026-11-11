//! Session list over REST and the live board WebSocket.
//! Public API: plan/research-base/CONTRACT.md §4.

use std::path::Path;

use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::broadcast::error::RecvError;

use crate::keeper::{BoardEvent, Keeper};
use crate::sep::PermissionMode;

pub fn routes(keeper: Keeper) -> Router {
    Router::new()
        .route("/api/sessions", get(list).post(create))
        .route("/ws/board", get(ws_board))
        .with_state(keeper)
}

async fn list(State(keeper): State<Keeper>) -> Response {
    match keeper.refresh().await {
        Ok(rows) => Json(json!({"sessions": rows})).into_response(),
        Err(e) => fail(StatusCode::BAD_GATEWAY, e),
    }
}

#[derive(Deserialize)]
struct CreateBody {
    cwd: String,
    worktree: Option<Value>,
    mode: Option<PermissionMode>,
    prompt: Option<String>,
}

async fn create(State(keeper): State<Keeper>, Json(body): Json<CreateBody>) -> Response {
    if body.worktree.is_some() {
        return fail(StatusCode::NOT_IMPLEMENTED, "worktree is not supported yet");
    }
    if !is_existing_dir(&body.cwd).await {
        return fail(
            StatusCode::BAD_REQUEST,
            "cwd must be an absolute existing directory",
        );
    }
    let meta = match body.mode {
        Some(PermissionMode::AlwaysApprove) => json!({"yoloMode": true}),
        Some(PermissionMode::Auto) => json!({"autoMode": true}),
        Some(PermissionMode::Ask) | None => json!({}),
    };
    let created = keeper
        .handle()
        .request(
            "session/new",
            json!({"cwd": body.cwd, "mcpServers": [], "_meta": meta}),
        )
        .await;
    let session_id = match created {
        Ok(res) => match res.get("sessionId").and_then(Value::as_str) {
            Some(id) => id.to_owned(),
            None => return fail(StatusCode::BAD_GATEWAY, "session/new returned no sessionId"),
        },
        Err(e) => return fail(StatusCode::BAD_GATEWAY, e),
    };
    keeper.adopt(&session_id, std::path::Path::new(&body.cwd));
    if let Err(e) = keeper.refresh().await {
        return fail(StatusCode::BAD_GATEWAY, e);
    }
    if let Some(text) = body.prompt.filter(|p| !p.is_empty()) {
        let handle = keeper.handle();
        let sid = session_id.clone();
        tokio::spawn(async move {
            let params =
                json!({"sessionId": sid.clone(), "prompt": [{"type": "text", "text": text}]});
            // Long turns outlast the request timeout; the outcome only goes to the log.
            if let Err(e) = handle.request("session/prompt", params).await {
                tracing::debug!(error = %e, session = %sid, "session/prompt ended");
            }
        });
    }
    Json(json!({"sessionId": session_id})).into_response()
}

async fn is_existing_dir(cwd: &str) -> bool {
    let path = Path::new(cwd);
    path.is_absolute() && tokio::fs::metadata(path).await.is_ok_and(|m| m.is_dir())
}

fn fail(status: StatusCode, error: impl std::fmt::Display) -> Response {
    (status, Json(json!({"error": error.to_string()}))).into_response()
}

async fn ws_board(State(keeper): State<Keeper>, ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(move |socket| board_socket(socket, keeper))
}

async fn board_socket(mut socket: WebSocket, keeper: Keeper) {
    let mut events = keeper.subscribe();
    if !send(&mut socket, &upsert(keeper.rows())).await {
        return;
    }
    loop {
        tokio::select! {
            event = events.recv() => {
                let frame = match event {
                    Ok(BoardEvent::Upsert(rows)) => upsert(rows),
                    Ok(BoardEvent::Remove(ids)) => json!({"t": "board.remove", "ids": ids}),
                    Err(RecvError::Lagged(skipped)) => {
                        tracing::debug!(skipped, "board client lagged; resending rows");
                        upsert(keeper.rows())
                    }
                    Err(RecvError::Closed) => break,
                };
                if !send(&mut socket, &frame).await {
                    break;
                }
            }
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Close(_)) | Err(_)) | None => break,
                Some(Ok(_)) => {}
            }
        }
    }
}

fn upsert(sessions: Vec<Value>) -> Value {
    json!({"t": "board.upsert", "sessions": sessions})
}

async fn send(socket: &mut WebSocket, frame: &Value) -> bool {
    socket
        .send(Message::Text(frame.to_string().into()))
        .await
        .is_ok()
}
