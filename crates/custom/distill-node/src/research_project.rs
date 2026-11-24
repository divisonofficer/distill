use axum::{
    Json, Router,
    body::Body,
    extract::{Path as AxumPath, Query, State},
    http::{HeaderValue, Method, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    research::ResearchService,
    research_refs::{ProjectResearchRefs, RefsUpdate},
};

#[derive(Clone)]
struct ProjectState(ResearchService);

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SettingsRequest {
    primary_metric: Option<String>,
    metric_directions: std::collections::BTreeMap<String, String>,
    metric_units: std::collections::BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct ArtifactQuery {
    path: String,
    #[serde(default)]
    download: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LinksRequest {
    literature_ids: Option<Vec<String>>,
    paper_ref: Option<Option<crate::research_refs::PaperRef>>,
    artifact_paths: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct ApprovalRequest {
    approved: bool,
}

pub(crate) fn routes(service: ResearchService) -> Router {
    Router::new()
        .route("/api/research/projects/{id}/settings", put(settings))
        .route(
            "/api/research/projects/{id}/experiments/{experiment}/links",
            put(links),
        )
        .route(
            "/api/research/projects/{id}/experiments/{experiment}/snapshot",
            get(snapshot),
        )
        .route(
            "/api/research/projects/{id}/experiments/{experiment}/workspace",
            post(workspace),
        )
        .route(
            "/api/research/projects/{id}/runs/{run}/metrics",
            get(metrics),
        )
        .route(
            "/api/research/projects/{id}/runs/{run}/reconnect",
            post(reconnect),
        )
        .route(
            "/api/research/projects/{id}/experiments/{experiment}/approvals",
            post(request_approval),
        )
        .route(
            "/api/research/projects/{id}/approvals/{approval}",
            get(get_approval).post(decide_approval),
        )
        .route("/api/research/projects/{id}/artifact", get(artifact))
        .with_state(ProjectState(service))
}

async fn settings(
    State(ProjectState(service)): State<ProjectState>,
    AxumPath(project): AxumPath<String>,
    Json(request): Json<SettingsRequest>,
) -> Response {
    if let Err(error) = service.owned_project(&project).await {
        return error;
    }
    let valid_metric = |s: &str| !s.trim().is_empty() && s.len() <= 128;
    if request
        .primary_metric
        .as_deref()
        .is_some_and(|s| !valid_metric(s))
        || request
            .metric_directions
            .iter()
            .any(|(key, value)| !valid_metric(key) || !matches!(value.as_str(), "higher" | "lower"))
        || request
            .metric_units
            .iter()
            .any(|(key, value)| !valid_metric(key) || value.len() > 64)
    {
        return error(StatusCode::BAD_REQUEST, "Invalid metric settings");
    }
    update_refs(
        &service,
        &project,
        RefsUpdate {
            settings: Some(crate::research_refs::ResearchSettings {
                primary_metric: request.primary_metric,
                metric_directions: request.metric_directions,
                metric_units: request.metric_units,
            }),
            ..Default::default()
        },
    )
    .await
}

async fn links(
    State(ProjectState(service)): State<ProjectState>,
    AxumPath((project, experiment)): AxumPath<(String, String)>,
    Json(request): Json<LinksRequest>,
) -> Response {
    if let Err(error) = service.owned_experiment(&project, &experiment).await {
        return error;
    }
    let refs = match refs_get(&service, &project).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let literature_ids = request.literature_ids;
    if literature_ids
        .iter()
        .flatten()
        .any(|id| !refs.literature.iter().any(|paper| &paper.id == id))
    {
        return error(
            StatusCode::BAD_REQUEST,
            "Literature link must reference a project paper",
        );
    }
    let artifact_paths = request.artifact_paths;
    if artifact_paths
        .as_ref()
        .is_some_and(|paths| paths.iter().any(|path| !safe_relative_path(path)))
    {
        return error(
            StatusCode::BAD_REQUEST,
            "Artifact path must be safe and relative",
        );
    }
    if artifact_paths
        .as_ref()
        .is_some_and(|paths| !paths.is_empty())
    {
        let listing = match service
            .native_json(
                Method::GET,
                &format!("api/projects/{}/files", enc(&project)),
                None,
            )
            .await
        {
            Ok(value) => value,
            Err(response) => return response,
        };
        let mut available = Vec::new();
        let mut pending = listing["entries"].as_array().cloned().unwrap_or_default();
        while let Some(item) = pending.pop() {
            if let Some(children) = item["children"].as_array() {
                pending.extend(children.iter().cloned());
            }
            if item["isDir"] != true {
                if let Some(path) = item["path"].as_str() {
                    available.push(path.to_owned());
                }
            }
        }
        if artifact_paths
            .as_ref()
            .unwrap()
            .iter()
            .any(|path| !available.contains(path))
        {
            return error(
                StatusCode::BAD_REQUEST,
                "Artifact link must reference an existing project file",
            );
        }
    }
    if let Some(Some(paper_ref)) = request.paper_ref.as_ref() {
        if !safe_relative_path(&paper_ref.path) {
            return error(
                StatusCode::BAD_REQUEST,
                "Paper reference path must be safe and relative",
            );
        }
        if !file_exists(&service, &project, &paper_ref.path).await {
            return error(
                StatusCode::BAD_REQUEST,
                "Paper reference must point to an existing project file",
            );
        }
    }
    match service
        .refs_modify(&project, move |refs| {
            let entry = refs.experiments.entry(experiment).or_default();
            if let Some(literature_ids) = literature_ids {
                entry.literature_ids = literature_ids;
            }
            if let Some(artifact_paths) = artifact_paths {
                entry.artifact_paths = artifact_paths;
            }
            if let Some(paper_ref) = request.paper_ref {
                entry.paper_ref = paper_ref;
            }
            Ok(())
        })
        .await
    {
        Ok(value) => Json(value).into_response(),
        Err(response) => response,
    }
}

async fn snapshot(
    State(ProjectState(service)): State<ProjectState>,
    AxumPath((project, experiment)): AxumPath<(String, String)>,
) -> Response {
    if let Err(error) = service.owned_experiment(&project, &experiment).await {
        return error;
    }
    proxy_json(
        &service,
        Method::GET,
        &format!(
            "api/v2/projects/{}/experiments/{}/snapshot",
            enc(&project),
            enc(&experiment)
        ),
        None,
    )
    .await
}

async fn workspace(
    State(ProjectState(service)): State<ProjectState>,
    AxumPath((project, experiment)): AxumPath<(String, String)>,
) -> Response {
    if let Err(error) = service.owned_experiment(&project, &experiment).await {
        return error;
    }
    proxy_json(
        &service,
        Method::POST,
        &format!(
            "api/v2/projects/{}/experiments/{}/workspace",
            enc(&project),
            enc(&experiment)
        ),
        Some(json!({})),
    )
    .await
}

async fn reconnect(
    State(ProjectState(service)): State<ProjectState>,
    AxumPath((project, run)): AxumPath<(String, String)>,
) -> Response {
    if let Err(error) = service.owned_run(&project, &run).await {
        return error;
    }
    match service
        .native_json(
            Method::POST,
            &format!("api/v2/runs/{}/reconnect", enc(&run)),
            Some(json!({})),
        )
        .await
    {
        Ok(run) => Json(json!({"run":run})).into_response(),
        Err(response) => response,
    }
}

async fn metrics(
    State(ProjectState(service)): State<ProjectState>,
    AxumPath((project, run)): AxumPath<(String, String)>,
) -> Response {
    match service.metrics(&project, &run).await {
        Ok(value) => Json(value).into_response(),
        Err(response) => response,
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ApprovalInput {
    expected_commit: String,
    target_id: String,
}
async fn request_approval(
    State(ProjectState(service)): State<ProjectState>,
    AxumPath((project, experiment)): AxumPath<(String, String)>,
    Json(input): Json<ApprovalInput>,
) -> Response {
    match service
        .request_run_approval(
            &project,
            &experiment,
            None,
            &input.target_id,
            &input.expected_commit,
        )
        .await
    {
        Ok(value) => Json(value).into_response(),
        Err(response) => response,
    }
}

async fn get_approval(
    State(ProjectState(service)): State<ProjectState>,
    AxumPath((project, approval)): AxumPath<(String, String)>,
) -> Response {
    if let Err(error) = service.owned_project(&project).await {
        return error;
    }
    let refs = match refs_get(&service, &project).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let Some(item) = refs
        .approvals
        .iter()
        .find(|a| a.id == approval && a.project_id == project)
    else {
        return error(StatusCode::NOT_FOUND, "Approval not found in this project");
    };
    let mut run = Value::Null;
    if item.status == "submitted" {
        if let Some(id) = item.run_id.as_deref() {
            match service.owned_run(&project, id).await {
                Ok(v) => run = v,
                Err(e) => return e,
            }
        }
    }
    Json(json!({"approval":item,"run":if run.is_null(){None}else{Some(run)}})).into_response()
}

async fn decide_approval(
    State(ProjectState(service)): State<ProjectState>,
    AxumPath((project, approval)): AxumPath<(String, String)>,
    Json(request): Json<ApprovalRequest>,
) -> Response {
    if let Err(error) = service.owned_project(&project).await {
        return error;
    }
    let refs = match refs_get(&service, &project).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let Some(item) = refs
        .approvals
        .iter()
        .find(|a| a.id == approval && a.project_id == project)
    else {
        return error(StatusCode::NOT_FOUND, "Approval not found in this project");
    };
    if item.status != "pending" {
        return error(StatusCode::CONFLICT, "Approval is no longer pending");
    }
    let result = service
        .execute_owner_approval(&project, &approval, request.approved)
        .await;
    match result {
        Ok(value) => Json(value).into_response(),
        Err(response) => response,
    }
}

async fn artifact(
    State(ProjectState(service)): State<ProjectState>,
    AxumPath(project): AxumPath<String>,
    Query(query): Query<ArtifactQuery>,
) -> Response {
    if let Err(error) = service.owned_project(&project).await {
        return error;
    }
    if !safe_relative_path(&query.path) {
        return error(
            StatusCode::BAD_REQUEST,
            "Artifact path must be safe and relative",
        );
    }
    if !file_exists(&service, &project, &query.path).await {
        return error(StatusCode::NOT_FOUND, "Artifact not found in this project");
    }
    let native_path = format!(
        "api/projects/{}/files/content?path={}",
        enc(&project),
        urlencoding::encode(&query.path)
    );
    let (mime, bytes) = match service.native_bytes(&native_path, 32 * 1024 * 1024).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let raster_image = mime.to_ascii_lowercase().starts_with("image/")
        && !mime.to_ascii_lowercase().contains("svg");
    let safe_mime = if raster_image {
        mime
    } else {
        "application/octet-stream".into()
    };
    let mut response = Body::from(bytes).into_response();
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&safe_mime)
            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
    );
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    let filename = query
        .path
        .rsplit('/')
        .next()
        .unwrap_or("artifact")
        .replace(['\\', '"', '\r', '\n'], "_");
    let disposition = if raster_image && query.download != "1" {
        "inline"
    } else {
        "attachment"
    };
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("{disposition}; filename=\"{filename}\"")).unwrap(),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

async fn file_exists(service: &ResearchService, project: &str, path: &str) -> bool {
    let listing = match service
        .native_json(
            Method::GET,
            &format!("api/projects/{}/files", enc(project)),
            None,
        )
        .await
    {
        Ok(value) => value,
        Err(_) => return false,
    };
    let mut pending = listing["entries"].as_array().cloned().unwrap_or_default();
    while let Some(item) = pending.pop() {
        if item["isDir"] == true {
            if let Some(children) = item["children"].as_array() {
                pending.extend(children.iter().cloned());
            }
        } else if item["path"].as_str() == Some(path) {
            return true;
        }
    }
    false
}

async fn proxy_json(
    service: &ResearchService,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> Response {
    match service.native_json(method, path, body).await {
        Ok(value) => Json(value).into_response(),
        Err(error) => error,
    }
}
async fn refs_get(
    service: &ResearchService,
    project: &str,
) -> Result<ProjectResearchRefs, Response> {
    service.refs_get(project).await
}
async fn update_refs(service: &ResearchService, project: &str, update: RefsUpdate) -> Response {
    match service.refs_update(project, update).await {
        Ok(value) => Json(value).into_response(),
        Err(response) => response,
    }
}
fn safe_relative_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 1024
        && !path.starts_with('/')
        && !path.contains('\\')
        && path.split('/').all(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && !part.bytes().any(|b| b.is_ascii_control())
        })
}
fn enc(value: &str) -> String {
    urlencoding::encode(value).into_owned()
}
fn error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({"error":message.into()}))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn artifact_paths_reject_traversal_and_accept_nested_paths() {
        assert!(safe_relative_path("results/chart.png"));
        assert!(!safe_relative_path("../secret"));
        assert!(!safe_relative_path("/etc/passwd"));
        assert!(!safe_relative_path("a\\b"));
    }
}
