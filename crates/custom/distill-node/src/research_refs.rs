use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

const MAX_REFS_BYTES: u64 = 1024 * 1024;
static TEMP_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProjectResearchRefs {
    pub literature: Vec<ResearchPaper>,
    pub experiments: BTreeMap<String, ExperimentRefs>,
    pub settings: ResearchSettings,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub approvals: Vec<ResearchApproval>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub target_bindings: Vec<Binding>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Binding {
    pub id: String,
    pub postudio_host_id: String,
    pub orx_target_alias: String,
    pub backend: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ResearchPaper {
    pub id: String,
    pub source: String,
    pub source_id: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authors: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub year: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub venue: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(rename = "abstract", skip_serializing_if = "Option::is_none")]
    pub abstract_text: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExperimentRefs {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub distill_session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_ids: Option<Vec<String>>,
    pub literature_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paper_ref: Option<PaperRef>,
    pub artifact_paths: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PaperRef {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ResearchSettings {
    pub primary_metric: Option<String>,
    pub metric_directions: BTreeMap<String, String>,
    pub metric_units: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ResearchApproval {
    pub id: String,
    pub project_id: String,
    pub experiment_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub expected_commit: String,
    pub target_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_spec: Option<serde_json::Value>,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RefsUpdate {
    pub literature: Option<Vec<ResearchPaper>>,
    pub experiments: Option<BTreeMap<String, ExperimentRefs>>,
    pub settings: Option<ResearchSettings>,
    pub approvals: Option<Vec<ResearchApproval>>,
}

#[derive(Clone)]
pub(crate) struct RefsStore {
    path: PathBuf,
    lock: Arc<Mutex<()>>,
}

impl RefsStore {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self {
            path: root.join("research/refs.json"),
            lock: Arc::new(Mutex::new(())),
        }
    }

    pub(crate) async fn get(&self, project_id: &str) -> Result<ProjectResearchRefs, String> {
        let _guard = self.lock.lock().await;
        let all = self.read().await?;
        Ok(all.get(project_id).cloned().unwrap_or_default())
    }

    pub(crate) async fn update(
        &self,
        project_id: &str,
        update: RefsUpdate,
    ) -> Result<ProjectResearchRefs, String> {
        self.modify(project_id, |refs| {
            if let Some(value) = update.literature {
                refs.literature = value;
            }
            if let Some(value) = update.experiments {
                refs.experiments = value;
            }
            if let Some(value) = update.settings {
                refs.settings = value;
            }
            if let Some(value) = update.approvals {
                refs.approvals = value;
            }
        })
        .await
    }

    pub(crate) async fn modify(
        &self,
        project_id: &str,
        modify: impl FnOnce(&mut ProjectResearchRefs),
    ) -> Result<ProjectResearchRefs, String> {
        let _guard = self.lock.lock().await;
        let mut all = self.read().await?;
        let mut refs = all.remove(project_id).unwrap_or_default();
        modify(&mut refs);
        all.insert(project_id.to_owned(), refs.clone());
        self.write(&all).await?;
        Ok(refs)
    }

    async fn read(&self) -> Result<BTreeMap<String, ProjectResearchRefs>, String> {
        let file = match tokio::fs::File::open(&self.path).await {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
            Err(e) => return Err(format!("Could not read research reference data: {e}")),
        };
        let metadata = file
            .metadata()
            .await
            .map_err(|e| format!("Could not inspect research reference data: {e}"))?;
        if metadata.len() > MAX_REFS_BYTES {
            return Err("Research references exceed size limit".into());
        }
        let mut data = Vec::with_capacity(metadata.len() as usize);
        use tokio::io::AsyncReadExt;
        file.take(MAX_REFS_BYTES + 1)
            .read_to_end(&mut data)
            .await
            .map_err(|e| format!("Could not read research reference data: {e}"))?;
        if data.len() as u64 > MAX_REFS_BYTES {
            return Err("Research references exceed size limit".into());
        }
        serde_json::from_slice(&data)
            .map_err(|e| format!("Research reference data is invalid: {e}"))
    }

    async fn write(&self, value: &BTreeMap<String, ProjectResearchRefs>) -> Result<(), String> {
        let parent = self
            .path
            .parent()
            .ok_or("Research reference path has no parent")?;
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("Could not prepare research reference store: {e}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))
                .await
                .map_err(|e| format!("Could not secure research reference store: {e}"))?;
        }
        let bytes = serde_json::to_vec(value)
            .map_err(|e| format!("Could not encode research references: {e}"))?;
        if bytes.len() as u64 > MAX_REFS_BYTES {
            return Err("Research references exceed size limit".into());
        }
        let (temp, mut file) = loop {
            let nonce = TEMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let name = format!("refs.{}.{}.tmp", std::process::id(), nonce);
            let path = parent.join(name);
            let mut options = tokio::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                options.mode(0o600);
            }
            match options.open(&path).await {
                Ok(file) => break (path, file),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => {
                    return Err(format!(
                        "Could not create research reference temp file: {e}"
                    ));
                }
            }
        };
        use tokio::io::AsyncWriteExt;
        if let Err(e) = file.write_all(&bytes).await {
            let _ = tokio::fs::remove_file(&temp).await;
            return Err(format!("Could not write research references: {e}"));
        }
        if let Err(e) = file.sync_all().await {
            drop(file);
            let _ = tokio::fs::remove_file(&temp).await;
            return Err(format!("Could not sync research references: {e}"));
        }
        drop(file);
        if let Err(e) = tokio::fs::rename(&temp, &self.path).await {
            let _ = tokio::fs::remove_file(&temp).await;
            return Err(format!("Could not commit research references: {e}"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abstract_serializes_with_api_field_name() {
        let paper = ResearchPaper {
            id: "x".into(),
            source: "alphaXiv".into(),
            source_id: "x".into(),
            title: "t".into(),
            authors: None,
            year: None,
            venue: None,
            url: None,
            abstract_text: Some("summary".into()),
        };
        let value = serde_json::to_value(paper).unwrap();
        assert_eq!(value["abstract"], "summary");
        assert!(value.get("abstractText").is_none());
    }
}
