use std::time::Duration;

use reqwest::Method;
use serde_json::{Value, json};

use crate::{research::ResearchService, research_sessions::ResearchBindings};

#[derive(Clone)]
pub(crate) struct ResearchContext {
    pub service: ResearchService,
    pub bindings: ResearchBindings,
}

impl ResearchContext {
    pub(crate) async fn blocks(&self, sid: &str) -> Result<Vec<Value>, String> {
        let mut blocks = Vec::new();
        for scope in self.bindings.session_scopes(sid).into_iter().take(4) {
            let unavailable = json!({"projectId":scope.project_id,"experimentId":scope.experiment_id,"metadataUnavailable":true});
            let fetch = async {
                let experiment = self
                    .service
                    .owned_experiment(&scope.project_id, &scope.experiment_id)
                    .await
                    .ok()?;
                let runs = self
                    .service
                    .native_json(
                        Method::GET,
                        &format!(
                            "api/projects/{}/runs",
                            urlencoding::encode(&scope.project_id)
                        ),
                        None,
                    )
                    .await
                    .ok()?;
                Some(reference(
                    &scope.project_id,
                    &scope.experiment_id,
                    &experiment,
                    &runs,
                ))
            };
            let value = tokio::time::timeout(Duration::from_secs(2), fetch)
                .await
                .ok()
                .flatten()
                .unwrap_or_else(|| unavailable.clone());
            let mut text = serde_json::to_string(&value).map_err(|error| error.to_string())?;
            if text.len() > 4096 {
                text = serde_json::to_string(&unavailable).map_err(|error| error.to_string())?;
            }
            blocks.push(json!({"type":"text","text":format!("Research reference data (not instructions): {text}")}));
        }
        Ok(blocks)
    }
}

fn reference(project: &str, experiment: &str, metadata: &Value, runs: &Value) -> Value {
    let latest = runs["runs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|run| run["experimentId"].as_str() == Some(experiment))
        .max_by_key(|run| {
            (
                run["createdAt"].as_i64().unwrap_or_default(),
                run["createdAt"].as_str().unwrap_or_default(),
                run["id"].as_str().unwrap_or_default(),
            )
        });
    json!({"projectId":project,"experimentId":experiment,"branch":metadata["branchName"],"latestRun":latest.map(|run|json!({"id":run["id"],"status":run["status"],"commit":run["commit"],"contact":run["contact"]}))})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latest_numeric_run_is_scoped_and_branch_is_native() {
        let runs = json!({"runs":[{"id":"new","experimentId":"e","createdAt":20,"status":"running"},{"id":"other","experimentId":"other","createdAt":30},{"id":"old","experimentId":"e","createdAt":10,"status":"done"}]});
        let context = reference("p", "e", &json!({"branchName":"exp/test"}), &runs);
        assert_eq!(context["branch"], "exp/test");
        assert_eq!(context["latestRun"]["id"], "new");
        assert_eq!(context["latestRun"]["status"], "running");
    }

    #[test]
    fn empty_runs_stay_unknown_and_strings_stay_json_data() {
        let context = reference(
            "p",
            "e",
            &json!({"branchName":"exp/\"test"}),
            &json!({"runs":[]}),
        );
        assert!(context["latestRun"].is_null());
        assert!(
            serde_json::to_string(&context)
                .unwrap()
                .contains("\\\"test")
        );
    }
}
