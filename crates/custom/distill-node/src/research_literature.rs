use std::{collections::BTreeMap, sync::Arc};

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{delete, get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::{
    research::ResearchService,
    research_refs::{ExperimentRefs, ResearchPaper},
};

const MAX_QUERY: usize = 512;
const SOURCES: [&str; 4] = ["alphaXiv", "openAlex", "bioRxiv", "pubMed"];

#[derive(Clone)]
struct LiteratureState {
    service: ResearchService,
    cache: Arc<Mutex<BTreeMap<(String, String), ResearchPaper>>>,
}

#[derive(Deserialize)]
struct PapersQuery {
    #[serde(default)]
    q: String,
    sources: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LinkRequest {
    paper: ResearchPaper,
    experiment_id: Option<String>,
}

pub(crate) fn routes(service: ResearchService) -> Router {
    let state = LiteratureState {
        service,
        cache: Arc::new(Mutex::new(BTreeMap::new())),
    };
    Router::new()
        .route("/api/research/projects/{id}/refs", get(refs_get))
        .route("/api/research/papers", get(papers))
        .route("/api/research/projects/{id}/literature", post(link_paper))
        .route(
            "/api/research/projects/{id}/literature/{paper_id}",
            delete(unlink_paper),
        )
        .with_state(state)
}

async fn refs_get(State(s): State<LiteratureState>, Path(project): Path<String>) -> Response {
    if let Err(response) = s.service.owned_project(&project).await {
        return response;
    }
    match s.service.refs_get(&project).await {
        Ok(refs) => Json(refs).into_response(),
        Err(response) => response,
    }
}

async fn papers(State(s): State<LiteratureState>, Query(query): Query<PapersQuery>) -> Response {
    let q = query.q.trim();
    if q.is_empty() || q.len() > MAX_QUERY {
        return error(
            StatusCode::BAD_REQUEST,
            "Search query must contain 1 to 512 bytes",
        );
    }
    let requested = match query.sources.as_deref() {
        Some(sources) => {
            let values: Vec<_> = sources
                .split(',')
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .collect();
            if values.iter().any(|value| !SOURCES.contains(value)) {
                return error(StatusCode::BAD_REQUEST, "Unsupported literature source");
            }
            if values.is_empty() {
                vec!["alphaXiv"]
            } else {
                values
            }
        }
        None => vec!["alphaXiv"],
    };

    // Four fixed futures cap concurrency independently of caller input.
    let (alpha, open, bio, pubmed) = tokio::join!(
        selected_search(&s.service, "alphaXiv", q, &requested),
        selected_search(&s.service, "openAlex", q, &requested),
        selected_search(&s.service, "bioRxiv", q, &requested),
        selected_search(&s.service, "pubMed", q, &requested),
    );
    let outcomes = [alpha, open, bio, pubmed];
    let mut result = Vec::new();
    let mut errors = BTreeMap::new();
    let mut cache = s.cache.lock().await;
    for (source, outcome) in SOURCES.iter().zip(outcomes) {
        if !requested.contains(source) {
            continue;
        }
        match outcome {
            Ok(value) => {
                let items = value.as_array().or_else(|| value["papers"].as_array());
                if let Some(items) = items {
                    for item in items {
                        if let Some(paper) = normalize(source, item) {
                            if cache.len() >= 1000 {
                                cache.clear();
                            }
                            cache.insert(
                                (paper.source.clone(), paper.source_id.clone()),
                                paper.clone(),
                            );
                            result.push(paper);
                        }
                    }
                } else {
                    errors.insert(
                        (*source).to_owned(),
                        "Invalid native discovery response".to_owned(),
                    );
                }
            }
            Err(message) => {
                errors.insert((*source).to_owned(), message);
            }
        }
    }
    Json(json!({"papers":result,"sourceErrors":errors})).into_response()
}

async fn selected_search(
    service: &ResearchService,
    source: &str,
    query: &str,
    requested: &[&str],
) -> Result<Value, String> {
    if !requested.contains(&source) {
        return Ok(json!([]));
    }
    search(service, source, query).await
}
async fn search(service: &ResearchService, source: &str, query: &str) -> Result<Value, String> {
    service
        .discover_papers(source, query)
        .await
        .map_err(|_| format!("{source} search failed"))
}

fn normalize(source: &str, item: &Value) -> Option<ResearchPaper> {
    let native = item.get("source").and_then(Value::as_str)?;
    let expected = match source {
        "alphaXiv" => "alphaxiv",
        "openAlex" => "openalex",
        "bioRxiv" => "biorxiv",
        "pubMed" => "pubmed",
        _ => return None,
    };
    if native != expected {
        return None;
    }
    let id = item.get("id")?.as_str()?.trim();
    let title = item.get("title")?.as_str()?.trim();
    if id.is_empty() || title.is_empty() || id.len() > 512 || title.len() > 8192 {
        return None;
    }
    let (canonical_id, url) = match source {
        "alphaXiv" => (
            id.to_owned(),
            format!("https://www.alphaxiv.org/abs/{}", urlencoding::encode(id)),
        ),
        "openAlex" => {
            let id = id.rsplit('/').next().unwrap_or(id);
            if !id.starts_with('W') || !id[1..].bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            (id.to_owned(), format!("https://openalex.org/{id}"))
        }
        "bioRxiv" => {
            if !id.starts_with("10.1101/") {
                return None;
            }
            (
                id.to_owned(),
                format!("https://doi.org/{}", urlencoding::encode(id)),
            )
        }
        "pubMed" => {
            if !id.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            (
                id.to_owned(),
                format!("https://pubmed.ncbi.nlm.nih.gov/{id}/"),
            )
        }
        _ => return None,
    };
    let year = item
        .get("publicationDate")
        .and_then(Value::as_str)
        .and_then(|date| date.get(..4))
        .and_then(|year| year.parse().ok());
    Some(ResearchPaper {
        id: format!("{source}:{canonical_id}"),
        source: source.to_owned(),
        source_id: canonical_id,
        title: title.to_owned(),
        authors: None,
        year,
        venue: None,
        url: Some(url),
        abstract_text: item
            .get("abstract")
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
            .map(str::to_owned),
    })
}

async fn link_paper(
    State(s): State<LiteratureState>,
    Path(project): Path<String>,
    Json(req): Json<LinkRequest>,
) -> Response {
    if let Err(response) = s.service.owned_project(&project).await {
        return response;
    }
    let cache = s.cache.lock().await;
    let cached = cache
        .get(&(req.paper.source.clone(), req.paper.source_id.clone()))
        .cloned();
    drop(cache);
    let Some(paper) = cached.filter(|cached| {
        serde_json::to_value(cached).ok() == serde_json::to_value(&req.paper).ok()
    }) else {
        return error(
            StatusCode::BAD_REQUEST,
            "Paper must match a current server search result",
        );
    };
    if let Some(experiment_id) = req.experiment_id.as_deref() {
        if let Err(response) = s.service.owned_experiment(&project, experiment_id).await {
            return response;
        }
    }
    match s
        .service
        .refs_modify(&project, |refs| {
            if !refs
                .literature
                .iter()
                .any(|existing| existing.id == paper.id)
            {
                refs.literature.push(paper.clone());
            }
            if let Some(experiment_id) = req.experiment_id {
                let links = refs
                    .experiments
                    .entry(experiment_id)
                    .or_insert_with(ExperimentRefs::default);
                if !links.literature_ids.contains(&paper.id) {
                    links.literature_ids.push(paper.id.clone());
                }
            }
            Ok(())
        })
        .await
    {
        Ok(refs) => Json(refs).into_response(),
        Err(response) => response,
    }
}

async fn unlink_paper(
    State(s): State<LiteratureState>,
    Path((project, paper_id)): Path<(String, String)>,
) -> Response {
    if let Err(response) = s.service.owned_project(&project).await {
        return response;
    }
    match s
        .service
        .refs_modify(&project, |refs| {
            refs.literature.retain(|paper| paper.id != paper_id);
            for experiment in refs.experiments.values_mut() {
                experiment.literature_ids.retain(|id| id != &paper_id);
            }
            Ok(())
        })
        .await
    {
        Ok(refs) => Json(refs).into_response(),
        Err(response) => response,
    }
}

fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({"error":message}))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normalizes_real_source_ids_and_rejects_mismatches() {
        let cases = [
            (
                "alphaXiv",
                json!({"source":"alphaxiv","id":"2401.12345","title":"T","abstract":"A"}),
                "https://www.alphaxiv.org/abs/2401.12345",
            ),
            (
                "openAlex",
                json!({"source":"openalex","id":"https://openalex.org/W123","title":"T"}),
                "https://openalex.org/W123",
            ),
            (
                "bioRxiv",
                json!({"source":"biorxiv","id":"10.1101/abc","title":"T"}),
                "https://doi.org/10.1101%2Fabc",
            ),
            (
                "pubMed",
                json!({"source":"pubmed","id":"12345","title":"T"}),
                "https://pubmed.ncbi.nlm.nih.gov/12345/",
            ),
        ];
        for (source, item, url) in cases {
            assert_eq!(normalize(source, &item).unwrap().url.as_deref(), Some(url));
        }
        assert!(
            normalize(
                "pubMed",
                &json!({"source":"openalex","id":"123","title":"T"})
            )
            .is_none()
        );
    }
}
