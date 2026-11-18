//! Session list over REST and the live board WebSocket.
//! Public API: plan/research-base/CONTRACT.md §4.

use std::path::{Component, Path, PathBuf};

use base64::Engine as _;
use ring::rand::{SecureRandom as _, SystemRandom};

use axum::Json;
use axum::Router;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{DefaultBodyLimit, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::broadcast::error::RecvError;

use crate::keeper::{BoardEvent, Keeper};
use crate::sep::{ImageIn, PermissionMode, TextFileIn};

pub fn routes(keeper: Keeper) -> Router {
    Router::new()
        .route(
            "/api/sessions",
            get(list)
                .post(create)
                .layer(DefaultBodyLimit::max(32 * 1024 * 1024)),
        )
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
    #[serde(rename = "modelId")]
    model_id: Option<String>,
    prompt: Option<String>,
    #[serde(default)]
    images: Vec<ImageIn>,
    #[serde(default)]
    text_files: Vec<TextFileIn>,
}

async fn create(State(keeper): State<Keeper>, Json(body): Json<CreateBody>) -> Response {
    let prompt_blocks = match crate::attachments::prompt_blocks(
        body.prompt.as_deref().filter(|text| !text.is_empty()),
        &body.images,
        &body.text_files,
    ) {
        Ok(blocks) => blocks,
        Err(message) => return fail(StatusCode::BAD_REQUEST, message),
    };
    if !is_existing_dir(&body.cwd).await {
        return fail(
            StatusCode::BAD_REQUEST,
            "cwd must be an absolute existing directory",
        );
    }
    if body
        .model_id
        .as_ref()
        .is_some_and(|id| id.trim().is_empty())
    {
        return fail(StatusCode::BAD_REQUEST, "modelId must not be empty");
    }
    let session_cwd = if let Some(worktree) = body.worktree.as_ref() {
        let name = worktree
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        if !valid_worktree_name(name) {
            return fail(
                StatusCode::BAD_REQUEST,
                "worktree name must be 1-64 letters, numbers, hyphens, or underscores",
            );
        }
        match create_worktree(&keeper, &body.cwd, name).await {
            Ok(path) => path,
            Err(e) => return fail(StatusCode::BAD_GATEWAY, e),
        }
    } else {
        body.cwd.clone()
    };
    let mode = body.mode.unwrap_or(PermissionMode::Ask);
    let mut meta = mode_meta(mode);
    if let Some(model_id) = body.model_id {
        meta["modelId"] = json!(model_id);
    }
    let created = keeper
        .handle()
        .request(
            "session/new",
            json!({"cwd": session_cwd, "mcpServers": [], "_meta": meta}),
        )
        .await;
    let session_id = match created {
        Ok(res) => match res.get("sessionId").and_then(Value::as_str) {
            Some(id) => id.to_owned(),
            None => return fail(StatusCode::BAD_GATEWAY, "session/new returned no sessionId"),
        },
        Err(e) => return fail(StatusCode::BAD_GATEWAY, e),
    };
    keeper.adopt(&session_id, Path::new(&session_cwd), mode);
    if let Err(e) = keeper.refresh().await {
        return fail(StatusCode::BAD_GATEWAY, e);
    }
    if !prompt_blocks.is_empty() {
        let handle = keeper.handle();
        let sid = session_id.clone();
        tokio::spawn(async move {
            let params = json!({"sessionId": sid.clone(), "prompt": prompt_blocks});
            // Long turns outlast the request timeout; the outcome only goes to the log.
            if let Err(e) = handle.request("session/prompt", params).await {
                tracing::debug!(error = %e, session = %sid, "session/prompt ended");
            }
        });
    }
    Json(json!({"sessionId": session_id})).into_response()
}

fn mode_meta(mode: PermissionMode) -> Value {
    match mode {
        PermissionMode::Ask => json!({"yoloMode": false, "autoMode": false}),
        PermissionMode::Auto => json!({"yoloMode": false, "autoMode": true}),
        PermissionMode::AlwaysApprove => json!({"yoloMode": true, "autoMode": false}),
    }
}

fn valid_worktree_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

async fn create_worktree(keeper: &Keeper, cwd: &str, name: &str) -> Result<String, String> {
    let mut random = [0u8; 18];
    SystemRandom::new()
        .fill(&mut random)
        .map_err(|_| "could not generate worktree correlation id".to_owned())?;
    let correlation = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random);
    let response = keeper
        .handle()
        .request(
            "_x.ai/git/worktree/create_from_worktree_sync",
            json!({
                "sourceWorktreePath": cwd,
                "newSessionId": correlation.clone(),
                "copyMode": "dirty",
                "label": name,
            }),
        )
        .await
        .map_err(|e| format!("worktree creation failed: {e}"))?;
    if let Some(error) = response.get("error").filter(|value| !value.is_null()) {
        return Err(match error {
            Value::String(message) => message.clone(),
            value => value.to_string(),
        });
    }
    let response = response
        .get("result")
        .filter(|value| !value.is_null())
        .ok_or_else(|| "worktree extension returned no result payload".to_owned())?;
    if response.get("status").and_then(Value::as_str) != Some("created") {
        return Err(format!(
            "worktree creation returned unexpected status: {}",
            response.get("status").unwrap_or(&Value::Null)
        ));
    }
    if response.get("newSessionId").and_then(Value::as_str) != Some(correlation.as_str()) {
        return Err("worktree creation returned a mismatched correlation id".to_owned());
    }
    let worktree_path = response
        .get("worktreePath")
        .and_then(Value::as_str)
        .ok_or_else(|| "worktree creation returned no worktreePath".to_owned())?;
    let worktree = Path::new(worktree_path);
    if !worktree.is_absolute()
        || worktree == Path::new(cwd)
        || !tokio::fs::metadata(worktree)
            .await
            .is_ok_and(|m| m.is_dir())
    {
        return Err("worktree creation returned an invalid directory".to_owned());
    }
    let source_root = response
        .get("sourceGitRoot")
        .and_then(Value::as_str)
        .ok_or_else(|| "worktree creation returned no sourceGitRoot".to_owned())?;
    let source_root = dunce::canonicalize(source_root)
        .map_err(|_| "sourceGitRoot is not an existing directory".to_owned())?;
    let source =
        dunce::canonicalize(cwd).map_err(|_| "cwd is not an existing directory".to_owned())?;
    let resolved_worktree = dunce::canonicalize(worktree)
        .map_err(|_| "worktree is not an existing directory".to_owned())?;
    if resolved_worktree == source_root {
        return Err("worktree creation returned the source repository".to_owned());
    }
    let relative = source
        .strip_prefix(&source_root)
        .map_err(|_| "sourceGitRoot does not contain cwd".to_owned())?;
    if relative
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err("cwd has invalid repository-relative path".to_owned());
    }
    let session_cwd: PathBuf = if relative.as_os_str().is_empty() {
        worktree.to_owned()
    } else {
        worktree.join(relative)
    };
    if !tokio::fs::metadata(&session_cwd)
        .await
        .is_ok_and(|m| m.is_dir())
        || !dunce::canonicalize(&session_cwd).is_ok_and(|path| path.starts_with(&resolved_worktree))
    {
        return Err("worktree does not contain the requested subdirectory".to_owned());
    }
    Ok(session_cwd.to_string_lossy().into_owned())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_session_mode_metadata_is_explicit() {
        assert_eq!(
            mode_meta(PermissionMode::Ask),
            json!({"yoloMode": false, "autoMode": false})
        );
        assert_eq!(
            mode_meta(PermissionMode::Auto),
            json!({"yoloMode": false, "autoMode": true})
        );
        assert_eq!(
            mode_meta(PermissionMode::AlwaysApprove),
            json!({"yoloMode": true, "autoMode": false})
        );
    }
}
