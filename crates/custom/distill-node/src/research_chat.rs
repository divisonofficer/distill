use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use axum::{
    Json, Router,
    extract::{Path as AxumPath, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    keeper::Keeper, research::ResearchService, research_sessions::ResearchBindings,
    sep::PermissionMode,
};

#[derive(Clone)]
pub(crate) struct ChatState {
    service: ResearchService,
    bindings: ResearchBindings,
    keeper: Keeper,
    mappings: Arc<Mutex<BTreeMap<String, String>>>,
    mapping_path: Arc<PathBuf>,
    gate: Arc<tokio::sync::Mutex<()>>,
    mapping_error: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChatRequest {
    request_id: String,
}

#[derive(Serialize, Deserialize, Default)]
struct MappingStore {
    sessions: BTreeMap<String, String>,
}

pub(crate) fn routes(
    service: ResearchService,
    bindings: ResearchBindings,
    keeper: Keeper,
) -> Router {
    let path = service.state_dir().join("research-chat/sessions.json");
    let (mappings, mapping_error) = match load_mappings(&path) {
        Ok(mappings) => (mappings, None),
        Err(error) => (BTreeMap::new(), Some(error.to_string())),
    };
    Router::new()
        .route(
            "/api/research/projects/{p}/experiments/{e}/chat",
            post(create_chat),
        )
        .with_state(ChatState {
            service,
            bindings,
            keeper,
            mappings: Arc::new(Mutex::new(mappings)),
            mapping_path: Arc::new(path),
            gate: Arc::new(tokio::sync::Mutex::new(())),
            mapping_error,
        })
}

async fn create_chat(
    State(state): State<ChatState>,
    AxumPath((project, experiment)): AxumPath<(String, String)>,
    Json(body): Json<ChatRequest>,
) -> Response {
    if let Some(error) = &state.mapping_error {
        return fail(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Could not load research chat mappings: {error}"),
        );
    }
    let request_id = body.request_id.trim();
    if request_id.is_empty() || request_id.len() > 256 {
        return fail(
            StatusCode::BAD_REQUEST,
            "requestId must contain 1-256 characters",
        );
    }
    let _gate = state.gate.lock().await;
    let key = format!("{}\0{}\0{}", project, experiment, request_id);
    if let Some(session_id) = state
        .mappings
        .lock()
        .ok()
        .and_then(|m| m.get(&key).cloned())
    {
        if session_id.is_empty() {
            return fail(
                StatusCode::CONFLICT,
                "Session creation outcome is unconfirmed; inspect existing sessions before retrying",
            );
        }
        let (session_id, grant) = session_id
            .split_once('\0')
            .map_or((session_id.as_str(), None), |(session, grant)| {
                (session, Some(grant))
            });
        if let Some(grant) = grant {
            if let Err(error) = state.bindings.bind(grant, session_id).await {
                return fail(StatusCode::INTERNAL_SERVER_ERROR, error.to_string());
            }
        }
        return response(&state, &project, &experiment, session_id).await;
    }
    let experiment_record = match state.service.owned_experiment(&project, &experiment).await {
        Ok(value) => value,
        Err(error) => return error,
    };
    let workspace = match state.service.workspace(&project, &experiment).await {
        Ok(value) => value,
        Err(error) => return error,
    };
    let Some(cwd) = workspace
        .get("cwd")
        .and_then(Value::as_str)
        .filter(|s| Path::new(s).is_absolute())
    else {
        return fail(
            StatusCode::BAD_GATEWAY,
            "Research workspace returned no absolute cwd",
        );
    };
    let refs = match state.service.refs_get(&project).await {
        Ok(value) => value,
        Err(error) => return error,
    };
    let pending = match state.bindings.create_pending(&project, &experiment).await {
        Ok(value) => value,
        Err(error) => return fail(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    };
    {
        let Ok(mut map) = state.mappings.lock() else {
            return fail(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Research mapping lock poisoned",
            );
        };
        map.insert(key.clone(), String::new());
        if let Err(error) = persist_mappings(&state.mapping_path, &map) {
            map.remove(&key);
            return fail(StatusCode::INTERNAL_SERVER_ERROR, error);
        }
    }
    let created = state
        .keeper
        .handle()
        .request(
            "session/new",
            json!({
                "cwd": cwd,
                "mcpServers": [pending.mcp_server],
                "_meta": {"yoloMode": false, "autoMode": false}
            }),
        )
        .await;
    let session_id = match created {
        Ok(value) => match value.get("sessionId").and_then(Value::as_str) {
            Some(id) => id.to_owned(),
            None => return fail(StatusCode::BAD_GATEWAY, "session/new returned no sessionId"),
        },
        Err(error) => return fail(StatusCode::BAD_GATEWAY, error),
    };
    {
        let Ok(mut map) = state.mappings.lock() else {
            return fail(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Research chat mapping lock poisoned",
            );
        };
        if let Err(error) =
            persist_created_session(&state.mapping_path, &mut map, key, &session_id, &pending.id)
        {
            return fail(StatusCode::INTERNAL_SERVER_ERROR, error);
        }
    }
    if let Err(error) = state.bindings.bind(&pending.id, &session_id).await {
        return fail(StatusCode::INTERNAL_SERVER_ERROR, error.to_string());
    }
    state
        .keeper
        .adopt(&session_id, Path::new(cwd), PermissionMode::Ask);
    if let Err(error) = state.keeper.refresh().await {
        return fail(StatusCode::BAD_GATEWAY, error.to_string());
    }
    if let Err(error) = state
        .service
        .refs_modify(&project, |refs| {
            let experiment_refs = refs.experiments.entry(experiment.clone()).or_default();
            experiment_refs
                .distill_session_id
                .get_or_insert_with(|| session_id.clone());
            let ids = experiment_refs.session_ids.get_or_insert_with(Vec::new);
            if !ids.contains(&session_id) {
                ids.push(session_id.clone());
            }
            Ok(())
        })
        .await
    {
        return error;
    }
    let mut result = chat_response(&state, &project, &experiment, &session_id);
    result["draftContext"] = context(&experiment_record, &refs, &experiment);
    Json(result).into_response()
}

async fn response(state: &ChatState, project: &str, experiment: &str, session: &str) -> Response {
    if let Err(error) = state
        .service
        .refs_modify(project, |refs| {
            let item = refs.experiments.entry(experiment.to_owned()).or_default();
            item.distill_session_id
                .get_or_insert_with(|| session.to_owned());
            let ids = item.session_ids.get_or_insert_with(Vec::new);
            if !ids.iter().any(|id| id == session) {
                ids.push(session.to_owned());
            }
            Ok(())
        })
        .await
    {
        return error;
    }
    let mut value = chat_response(state, project, experiment, session);
    match state.service.owned_experiment(project, experiment).await {
        Ok(record) => match state.service.refs_get(project).await {
            Ok(refs) => value["draftContext"] = context(&record, &refs, experiment),
            Err(error) => return error,
        },
        Err(error) => return error,
    }
    Json(value).into_response()
}

fn persist_created_session(
    path: &Path,
    map: &mut BTreeMap<String, String>,
    key: String,
    session: &str,
    grant: &str,
) -> Result<(), String> {
    map.insert(key, format!("{session}\0{grant}"));
    persist_mappings(path, map)
}

fn chat_response(_state: &ChatState, _project: &str, _experiment: &str, session: &str) -> Value {
    json!({"sessionId": session, "actualSessionId": session, "host": gethostname::gethostname().to_string_lossy()})
}

fn context(
    experiment: &Value,
    refs: &crate::research_refs::ProjectResearchRefs,
    id: &str,
) -> Value {
    let papers: Vec<Value> = refs
        .experiments
        .get(id)
        .map(|item| {
            item.literature_ids
                .iter()
                .filter_map(|paper_id| refs.literature.iter().find(|paper| paper.id == *paper_id))
                .filter_map(|paper| serde_json::to_value(paper).ok())
                .collect()
        })
        .unwrap_or_default();
    json!({"experiment": experiment, "literature": papers})
}

fn load_mappings(path: &Path) -> std::io::Result<BTreeMap<String, String>> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice::<MappingStore>(&bytes)
            .map(|s| s.sessions)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(error) => Err(error),
    }
}

fn persist_mappings(path: &Path, mappings: &BTreeMap<String, String>) -> Result<(), String> {
    let parent = path.parent().ok_or("mapping path has no parent")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
    }
    let bytes = serde_json::to_vec(&MappingStore {
        sessions: mappings.clone(),
    })
    .map_err(|e| e.to_string())?;
    let temp = parent.join(format!("sessions.{}.tmp", std::process::id()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temp).map_err(|e| e.to_string())?;
    use std::io::Write;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    fs::rename(temp, path).map_err(|e| e.to_string())
}

fn fail(status: StatusCode, message: impl ToString) -> Response {
    (status, message.to_string()).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_session_survives_failed_mapping_write() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("not-a-file");
        fs::create_dir(&path).unwrap();
        let mut map = BTreeMap::new();
        assert!(
            persist_created_session(&path, &mut map, "request".into(), "session", "grant").is_err()
        );
        assert_eq!(map["request"], "session\0grant");
    }
    #[test]
    fn mappings_survive_reload_and_keep_request_scope() {
        let root = std::env::temp_dir().join(format!("research-chat-test-{}", std::process::id()));
        let path = root.join("sessions.json");
        let mut entries = BTreeMap::new();
        entries.insert("p\0e\0req".to_owned(), "sid".to_owned());
        persist_mappings(&path, &entries).unwrap();
        assert_eq!(load_mappings(&path).unwrap(), entries);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn draft_context_does_not_emit_a_prompt() {
        let experiment = json!({"title":"test", "description":"measure"});
        let refs = crate::research_refs::ProjectResearchRefs::default();
        let draft = context(&experiment, &refs, "e");
        assert_eq!(draft["experiment"]["description"], "measure");
        assert_eq!(draft["literature"], json!([]));
    }
}
