//! Registry-backed subagent and command execution endpoints.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{Value, json};

use crate::keeper::Keeper;

pub fn routes(keeper: Keeper) -> Router {
    Router::new()
        .route("/api/sessions/{id}/subagents", get(list_agents))
        .route("/api/sessions/{id}/subagents/{agent_id}", get(get_agent))
        .route(
            "/api/sessions/{id}/subagents/{agent_id}/cancel",
            post(cancel_agent),
        )
        .route("/api/sessions/{id}/tasks", get(list_tasks))
        .route("/api/sessions/{id}/tasks/{task_id}/stop", post(stop_task))
        .with_state(keeper)
}

async fn session(keeper: &Keeper, id: &str) -> Result<(), Response> {
    keeper
        .attach(id)
        .await
        .map_err(|_| fail(StatusCode::NOT_FOUND, "session not found"))
}

async fn rpc(keeper: &Keeper, id: &str, method: &str, extra: Value) -> Result<Value, Response> {
    session(keeper, id).await?;
    let mut params = extra.as_object().cloned().unwrap_or_default();
    params.insert("sessionId".into(), json!(id));
    let response = keeper
        .handle()
        .request(method, Value::Object(params))
        .await
        .map_err(|e| fail(StatusCode::BAD_GATEWAY, e))?;
    let envelope = response.get("result").unwrap_or(&response);
    if let Some(error) = envelope.get("error").filter(|v| !v.is_null()) {
        return Err(fail(StatusCode::BAD_GATEWAY, error));
    }
    Ok(envelope.get("result").unwrap_or(envelope).clone())
}

async fn list_agents(State(keeper): State<Keeper>, Path(id): Path<String>) -> Response {
    match rpc(
        &keeper,
        &id,
        "_x.ai/subagent/list_running",
        json!({"includeCompleted": true}),
    )
    .await
    {
        Ok(value) => {
            let rows = value
                .get("subagents")
                .or_else(|| value.get("agents"))
                .and_then(Value::as_array);
            let Some(rows) = rows else {
                return fail(StatusCode::BAD_GATEWAY, "invalid subagent roster");
            };
            let subagents: Option<Vec<Value>> = rows
                .iter()
                .map(|row| {
                    let row = row.clone();
                    if row.get("parentSessionId").and_then(Value::as_str) != Some(&id)
                        || !valid_agent(&row, true)
                    {
                        return None;
                    }
                    Some(row)
                })
                .collect();
            match subagents {
                Some(subagents) => Json(json!({"subagents": subagents})).into_response(),
                None => fail(StatusCode::BAD_GATEWAY, "invalid subagent roster"),
            }
        }
        Err(response) => response,
    }
}

async fn get_agent(
    State(keeper): State<Keeper>,
    Path((id, agent_id)): Path<(String, String)>,
) -> Response {
    match rpc(
        &keeper,
        &id,
        "_x.ai/subagent/get",
        json!({"subagentId": agent_id}),
    )
    .await
    {
        Ok(value) => {
            let snapshot = value.get("snapshot").unwrap_or(&value);
            if snapshot.is_null()
                || snapshot.get("subagentId").and_then(Value::as_str) != Some(&agent_id)
                || snapshot.get("parentSessionId").and_then(Value::as_str) != Some(&id)
            {
                return fail(StatusCode::NOT_FOUND, "subagent not found");
            }
            if !valid_agent(snapshot, true) {
                return fail(StatusCode::BAD_GATEWAY, "invalid subagent snapshot");
            }
            Json(json!({"snapshot": snapshot})).into_response()
        }
        Err(response) => response,
    }
}

async fn cancel_agent(
    State(keeper): State<Keeper>,
    Path((id, agent_id)): Path<(String, String)>,
) -> Response {
    match rpc(
        &keeper,
        &id,
        "_x.ai/subagent/get",
        json!({"subagentId": agent_id}),
    )
    .await
    {
        Ok(value) => {
            let snapshot = value.get("snapshot").unwrap_or(&value);
            if snapshot.get("subagentId").and_then(Value::as_str) != Some(&agent_id)
                || snapshot.get("parentSessionId").and_then(Value::as_str) != Some(&id)
            {
                return fail(StatusCode::NOT_FOUND, "subagent not found");
            }
            if !valid_agent(snapshot, true) {
                return fail(StatusCode::BAD_GATEWAY, "invalid subagent snapshot");
            }
            let status = snapshot["status"].as_str().unwrap();
            if matches!(status, "completed" | "failed" | "cancelled") {
                return Json(json!({"subagentId":agent_id,"cancelled":false,"outcome":{"kind":"already_finished","status":status}})).into_response();
            }
            match rpc(
                &keeper,
                &id,
                "_x.ai/subagent/cancel",
                json!({"subagentId": agent_id}),
            )
            .await
            {
                Ok(result) if valid_cancel(&result, &agent_id) => Json(result).into_response(),
                Ok(_) => fail(StatusCode::BAD_GATEWAY, "invalid cancel response"),
                Err(response) => response,
            }
        }
        Err(response) => response,
    }
}

async fn list_tasks(State(keeper): State<Keeper>, Path(id): Path<String>) -> Response {
    match rpc(&keeper, &id, "_x.ai/task/list", json!({})).await {
        Ok(value) => {
            let rows = value.get("tasks").and_then(Value::as_array);
            let Some(rows) = rows else {
                return fail(StatusCode::BAD_GATEWAY, "invalid task registry response");
            };
            let can_stop = value.get("scopedKillSupported").and_then(Value::as_bool) == Some(true);
            let tasks: Option<Vec<Value>> = rows
                .iter()
                .filter(|row| owned_background(row, &id))
                .map(|row| normalize_task(row, can_stop))
                .collect();
            match tasks {
                Some(tasks) => Json(json!({"tasks": tasks})).into_response(),
                None => fail(StatusCode::BAD_GATEWAY, "invalid task registry response"),
            }
        }
        Err(response) => response,
    }
}

async fn stop_task(
    State(keeper): State<Keeper>,
    Path((id, task_id)): Path<(String, String)>,
) -> Response {
    match rpc(&keeper, &id, "_x.ai/task/list", json!({})).await {
        Ok(value) => {
            let Some(tasks) = value.get("tasks").and_then(Value::as_array) else {
                return fail(StatusCode::BAD_GATEWAY, "invalid task registry response");
            };
            let can_stop = value.get("scopedKillSupported").and_then(Value::as_bool) == Some(true);
            let Some(tasks) = tasks
                .iter()
                .filter(|row| owned_background(row, &id))
                .map(|row| normalize_task(row, can_stop))
                .collect::<Option<Vec<_>>>()
            else {
                return fail(StatusCode::BAD_GATEWAY, "invalid task registry response");
            };
            let Some(task) = tasks
                .iter()
                .find(|task| task.get("taskId").and_then(Value::as_str) == Some(&task_id))
            else {
                return fail(StatusCode::NOT_FOUND, "task not found");
            };
            if task.get("completed").and_then(Value::as_bool) == Some(true) {
                return Json(json!({"taskId": task_id, "outcome":"already_exited"}))
                    .into_response();
            }
            if !can_stop {
                return fail(StatusCode::CONFLICT, "safe scoped task stop unsupported");
            }
            match rpc(
                &keeper,
                &id,
                "_x.ai/task/kill_scoped",
                json!({"taskId": task_id, "source":"clientUi"}),
            )
            .await
            {
                Ok(result)
                    if result.get("taskId").and_then(Value::as_str) == Some(&task_id)
                        && matches!(
                            result.get("outcome").and_then(Value::as_str),
                            Some("killed" | "already_exited" | "not_found")
                        ) =>
                {
                    Json(result).into_response()
                }
                Ok(_) => fail(StatusCode::BAD_GATEWAY, "invalid kill response"),
                Err(response) => response,
            }
        }
        Err(response) => response,
    }
}

fn field<'a>(v: &'a Value, snake: &'static str, camel: &'static str) -> Option<&'a Value> {
    v.get(snake).or_else(|| v.get(camel))
}
fn valid_agent(v: &Value, status: bool) -> bool {
    [
        "subagentId",
        "parentSessionId",
        "childSessionId",
        "subagentType",
    ]
    .iter()
    .all(|k| {
        v.get(k)
            .and_then(Value::as_str)
            .is_some_and(|s| !s.is_empty())
    }) && v.get("description").and_then(Value::as_str).is_some()
        && ["startedAtEpochMs", "durationMs"]
            .iter()
            .all(|k| v.get(k).and_then(Value::as_u64).is_some())
        && v.get("toolCallId")
            .is_none_or(|id| id.as_str().is_some_and(|id| !id.is_empty()))
        && (!status
            || matches!(
                v.get("status").and_then(Value::as_str),
                Some("initializing" | "running" | "completed" | "failed" | "cancelled")
            ))
}
fn valid_cancel(v: &Value, id: &str) -> bool {
    if v.get("subagentId").and_then(Value::as_str) != Some(id) {
        return false;
    }
    let cancelled = v.get("cancelled").and_then(Value::as_bool);
    match v.pointer("/outcome/kind").and_then(Value::as_str) {
        Some("cancelled") => cancelled == Some(true),
        Some("not_found") => cancelled == Some(false),
        Some("already_finished") => {
            cancelled == Some(false)
                && matches!(
                    v.pointer("/outcome/status").and_then(Value::as_str),
                    Some("completed" | "failed" | "cancelled")
                )
        }
        _ => false,
    }
}
fn owned_background(task: &Value, id: &str) -> bool {
    field(task, "owner_session_id", "ownerSessionId").and_then(Value::as_str) == Some(id)
        && field(task, "is_backgrounded", "isBackgrounded").and_then(Value::as_bool) == Some(true)
}
fn normalize_task(task: &Value, can_stop: bool) -> Option<Value> {
    let started_ms = epoch_ms(field(task, "start_time", "startTime")?)?;
    let end = field(task, "end_time", "endTime")?;
    let ended = if end.is_null() {
        None
    } else {
        Some(epoch_ms(end)?)
    };
    let id = field(task, "task_id", "taskId")?
        .as_str()
        .filter(|s| !s.is_empty())?;
    let command = field(task, "display_command", "displayCommand")
        .and_then(Value::as_str)
        .or_else(|| task.get("command").and_then(Value::as_str))?;
    let cwd = task.get("cwd")?.as_str()?;
    let output = task.get("output")?.as_str()?;
    let output_file = field(task, "output_file", "outputFile")?.as_str()?;
    let mut truncated = task.get("truncated")?.as_bool()?;
    let completed = task.get("completed")?.as_bool()?;
    let exit = field(task, "exit_code", "exitCode")?;
    if !exit.is_null() && exit.as_i64().is_none() {
        return None;
    }
    let signal = task.get("signal")?;
    if !signal.is_null() && signal.as_str().is_none() {
        return None;
    }
    let kind = task.get("kind").and_then(Value::as_str)?;
    if !matches!(kind, "bash" | "monitor") {
        return None;
    }
    let total = field(task, "output_total_bytes", "outputTotalBytes")?.as_u64()?;
    const LIMIT: usize = 1024 * 1024;
    let output = if output.len() > LIMIT {
        truncated = true;
        let mut end = LIMIT;
        while !output.is_char_boundary(end) {
            end -= 1;
        }
        &output[..end]
    } else {
        output
    };
    let description = match task.get("description") {
        None | Some(Value::Null) => None,
        Some(value) => Some(value.as_str()?),
    };
    let mut result = json!({"taskId":id,"command":command,"cwd":cwd,"startedAtUnixMs":started_ms,"endedAtUnixMs":ended,"output":output,"outputFile":output_file,"truncated":truncated,"exitCode":exit,"signal":signal,"completed":completed,"kind":kind,"outputTotalBytes":total,"canStop":can_stop});
    if let Some(description) = description {
        result["description"] = json!(description);
    }
    Some(result)
}
fn epoch_ms(v: &Value) -> Option<u64> {
    let secs = v.get("secs_since_epoch")?.as_u64()?;
    let nanos = v.get("nanos_since_epoch")?.as_u64()?;
    if nanos >= 1_000_000_000 {
        return None;
    }
    secs.checked_mul(1000)?.checked_add(nanos / 1_000_000)
}
fn fail(status: StatusCode, error: impl std::fmt::Display) -> Response {
    (status, Json(json!({"error": error.to_string()}))).into_response()
}
