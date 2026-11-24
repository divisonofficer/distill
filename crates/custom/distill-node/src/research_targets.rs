use axum::{
    Json, Router,
    extract::{Path, State},
    http::{Method, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::research::ResearchService;
use crate::research_refs::Binding;

pub(crate) fn routes(service: ResearchService) -> Router {
    Router::new()
        .route("/api/research/projects/{id}/targets", get(list).put(save))
        .with_state(service)
}

async fn list(State(service): State<ResearchService>, Path(project): Path<String>) -> Response {
    if let Err(response) = service.owned_project(&project).await {
        return response;
    }
    let refs = match service.refs_get(&project).await {
        Ok(refs) => refs,
        Err(response) => return response,
    };
    let (aliases, source_error) = candidates(&service).await;
    let targets =
        std::iter::once(json!({"id":"local","label":"로컬","backend":"local","available":true}))
            .chain(refs.target_bindings.iter().map(|binding| {
                json!({
                    "id": binding.id,
                    "label": binding.postudio_host_id,
                    "backend": binding.backend,
                    "available": aliases.iter().any(|alias| alias == &binding.orx_target_alias),
                })
            }))
            .collect::<Vec<_>>();
    let mut value = json!({"targets":targets,"bindings":refs.target_bindings,"candidates":aliases.iter().map(|alias| json!({"alias":alias,"label":alias})).collect::<Vec<_>>()});
    if let Some(error) = source_error {
        value["sourceError"] = json!(error);
    }
    Json(value).into_response()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SaveRequest {
    bindings: Vec<Binding>,
}

async fn save(
    State(service): State<ResearchService>,
    Path(project): Path<String>,
    Json(input): Json<SaveRequest>,
) -> Response {
    if let Err(response) = service.owned_project(&project).await {
        return response;
    }
    let (aliases, source_error) = candidates(&service).await;
    if let Some(error) = validate(&input.bindings, &aliases) {
        return (StatusCode::BAD_REQUEST, Json(json!({"error":error}))).into_response();
    }
    if let Some(error) = source_error {
        if !input.bindings.is_empty() {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({"error":error})),
            )
                .into_response();
        }
    }
    let updated = match service.save_target_bindings(&project, input.bindings).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let _ = updated;
    list(State(service), Path(project)).await
}

async fn candidates(service: &ResearchService) -> (Vec<String>, Option<String>) {
    match service
        .native_json(Method::GET, "api/settings/ssh", None)
        .await
    {
        Ok(value) => (
            value
                .get("hosts")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|host| host.get("host").and_then(Value::as_str).map(str::to_owned))
                .collect(),
            None,
        ),
        Err(_) => (
            Vec::new(),
            Some("Native SSH target configuration unavailable".into()),
        ),
    }
}

pub(crate) async fn resolve(
    service: &ResearchService,
    project: &str,
    target_id: &str,
) -> Result<Value, Response> {
    service.owned_project(project).await?;
    if target_id == "local" {
        return Ok(json!({"backend":"local","requiresApproval":false}));
    }
    let refs = service.refs_get(project).await?;
    let binding = refs
        .target_bindings
        .iter()
        .find(|binding| binding.id == target_id)
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(json!({"error":"Research target not found"})),
            )
                .into_response()
        })?;
    let native = service
        .native_json(Method::GET, "api/settings/ssh", None)
        .await?;
    let host = native["hosts"]
        .as_array()
        .and_then(|items| {
            items
                .iter()
                .find(|host| host["host"].as_str() == Some(&binding.orx_target_alias))
        })
        .ok_or_else(|| {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({"error":"Configured native SSH alias unavailable"})),
            )
                .into_response()
        })?;
    Ok(
        json!({"backend":"ssh","host":binding.orx_target_alias,"postudioHostId":binding.postudio_host_id,"requiresApproval":true,"hostSpec":{"hostname":host["hostname"].as_str().unwrap_or(&binding.orx_target_alias),"user":host["user"],"port":host["port"],"container":host["container"],"remoteRoot":host["remoteRoot"]}}),
    )
}

fn validate(bindings: &[Binding], aliases: &[String]) -> Option<String> {
    if bindings.len() > 32 {
        return Some("At most 32 research target bindings allowed".into());
    }
    let mut ids = std::collections::HashSet::new();
    for binding in bindings {
        if binding.backend != "ssh" {
            return Some("Only ssh bindings allowed".into());
        }
        if binding.id == "local"
            || binding.id.is_empty()
            || binding.id.len() > 64
            || !binding
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            || !ids.insert(binding.id.as_str())
        {
            return Some("Invalid or duplicate research target id".into());
        }
        if !valid_metadata(&binding.postudio_host_id) {
            return Some("Invalid Postudio host reference".into());
        }
        if binding.orx_target_alias.is_empty()
            || binding.orx_target_alias.starts_with('-')
            || binding
                .orx_target_alias
                .chars()
                .any(|c| c.is_control() || c.is_whitespace())
            || binding.orx_target_alias.len() > 256
            || !aliases
                .iter()
                .any(|alias| alias == &binding.orx_target_alias)
        {
            return Some("SSH alias must match a configured native target".into());
        }
    }
    None
}

fn valid_metadata(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn binding(id: &str, alias: &str) -> Binding {
        Binding {
            id: id.into(),
            postudio_host_id: "owner-host".into(),
            orx_target_alias: alias.into(),
            backend: "ssh".into(),
        }
    }
    #[test]
    fn validates_saved_owner_bound_targets() {
        assert_eq!(
            validate(&[binding("node_1", "native")], &["native".into()]),
            None
        );
        assert!(validate(&[binding("local", "native")], &["native".into()]).is_some());
        assert!(validate(&[binding("node", "other")], &["native".into()]).is_some());
        assert!(validate(&vec![binding("node", "native"); 33], &["native".into()]).is_some());
        assert!(!valid_metadata("bad\nhost"));
    }
}
