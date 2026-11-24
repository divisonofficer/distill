use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, anyhow, bail};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ring::{
    hmac,
    rand::{SecureRandom, SystemRandom},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const TTL_SECS: u64 = 24 * 60 * 60;
static TEMP_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[derive(Clone, Serialize, Deserialize)]
pub(crate) enum NodeEndpoint {
    Http(String),
    Unix(PathBuf),
}

#[derive(Clone, Debug)]
pub(crate) struct ResearchScope {
    pub project_id: String,
    pub experiment_id: String,
    pub session_id: Option<String>,
    pub grant_id: String,
}

#[derive(Clone, Serialize, Deserialize)]
struct Grant {
    project_id: String,
    experiment_id: String,
    session_id: Option<String>,
    capability_hash: String,
    created_at: u64,
    expires_at: u64,
    revoked: bool,
    #[serde(default)]
    request_ids: Vec<String>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct Store {
    grants: BTreeMap<String, Grant>,
}

#[derive(Clone)]
pub(crate) struct ResearchBindings {
    key: Arc<Vec<u8>>,
    path: Arc<PathBuf>,
    state: Arc<Mutex<Store>>,
    endpoint: NodeEndpoint,
    mcp_exe: Arc<PathBuf>,
}

pub(crate) struct PendingGrant {
    pub id: String,
    pub mcp_server: Value,
}

impl ResearchBindings {
    pub(crate) fn open(state_dir: &Path, endpoint: NodeEndpoint) -> Result<Self> {
        let dir = state_dir.join("research-sessions");
        fs::create_dir_all(&dir).context("create research session directory")?;
        #[cfg(unix)]
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        #[cfg(unix)]
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
        let key_path = dir.join("hmac.key");
        let key = match fs::read(&key_path) {
            Ok(key) if key.len() == 32 => key,
            Ok(_) => bail!("invalid research HMAC key"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let mut key = vec![0; 32];
                SystemRandom::new()
                    .fill(&mut key)
                    .map_err(|_| anyhow!("secure random generation failed"))?;
                let mut opts = fs::OpenOptions::new();
                opts.write(true).create_new(true);
                #[cfg(unix)]
                opts.mode(0o600);
                let mut f = opts.open(&key_path).context("create research HMAC key")?;
                use std::io::Write;
                f.write_all(&key)?;
                f.sync_all()?;
                key
            }
            Err(e) => return Err(e).context("read research HMAC key"),
        };
        #[cfg(unix)]
        fs::set_permissions(&key_path, fs::Permissions::from_mode(0o600))?;
        let path = dir.join("grants.json");
        let state = match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).context("read research grants")?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Store::default(),
            Err(e) => return Err(e).context("read research grants"),
        };
        let mcp_exe = std::env::var_os("POSTUDIO_RESEARCH_MCP_BIN")
            .map(PathBuf::from)
            .map(Ok)
            .unwrap_or_else(std::env::current_exe)
            .context("resolve research MCP executable")?;
        let mcp_exe = Arc::new(
            mcp_exe
                .canonicalize()
                .context("resolve research MCP executable path")?,
        );
        if !mcp_exe.is_file() {
            bail!("Research MCP executable must be a file");
        }
        Ok(Self {
            key: Arc::new(key),
            path: Arc::new(path),
            state: Arc::new(Mutex::new(state)),
            endpoint,
            mcp_exe,
        })
    }

    pub(crate) async fn create_pending(
        &self,
        project_id: &str,
        experiment_id: &str,
    ) -> Result<PendingGrant> {
        validate_id(project_id)?;
        validate_id(experiment_id)?;
        let mut raw = [0; 24];
        SystemRandom::new()
            .fill(&mut raw)
            .map_err(|_| anyhow!("secure random generation failed"))?;
        let id = URL_SAFE_NO_PAD.encode(raw);
        let token = self.token(&id);
        let now = now()?;
        let grant = Grant {
            project_id: project_id.into(),
            experiment_id: experiment_id.into(),
            session_id: None,
            capability_hash: self.capability_hash(&token),
            created_at: now,
            expires_at: now + TTL_SECS,
            revoked: false,
            request_ids: vec![],
        };
        {
            let mut state = self
                .state
                .lock()
                .map_err(|_| anyhow!("research grants lock poisoned"))?;
            state.grants.insert(id.clone(), grant);
            self.persist(&state)?;
        }
        let exe = &*self.mcp_exe;
        let mut args = vec![json!("research-mcp")];
        match &self.endpoint {
            NodeEndpoint::Http(url) => {
                args.extend([json!("--node-url"), json!(url)]);
            }
            NodeEndpoint::Unix(path) => {
                args.extend([json!("--node-socket"), json!(path)]);
            }
        }
        args.extend([json!("--grant-id"), json!(id)]);
        Ok(PendingGrant {
            id,
            mcp_server: json!({
                "name":"postudio-research", "command":exe.to_string_lossy(), "args":args,
                "env":[{"name":"POSTUDIO_RESEARCH_TOKEN","value":token}]
            }),
        })
    }

    pub(crate) async fn bind(&self, grant_id: &str, session_id: &str) -> Result<()> {
        validate_id(grant_id)?;
        validate_id(session_id)?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow!("research grants lock poisoned"))?;
        let mut next = state.clone();
        let grant = next
            .grants
            .get_mut(grant_id)
            .ok_or_else(|| anyhow!("unknown research grant"))?;
        ensure_live(grant)?;
        match &grant.session_id {
            Some(existing) if existing == session_id => return Ok(()),
            Some(_) => bail!("research grant already bound"),
            None => grant.session_id = Some(session_id.to_owned()),
        }
        self.persist(&next)?;
        *state = next;
        Ok(())
    }

    pub(crate) async fn authorize(
        &self,
        grant_id: &str,
        token: &str,
        allow_pending: bool,
    ) -> Result<ResearchScope> {
        validate_id(grant_id)?;
        if token.is_empty() || token.len() > 256 {
            bail!("invalid research capability");
        }
        let state = self
            .state
            .lock()
            .map_err(|_| anyhow!("research grants lock poisoned"))?;
        let grant = state
            .grants
            .get(grant_id)
            .ok_or_else(|| anyhow!("unknown research grant"))?;
        ensure_live(grant)?;
        let expected = URL_SAFE_NO_PAD
            .decode(&grant.capability_hash)
            .context("invalid stored capability")?;
        hmac::verify(
            &hmac::Key::new(hmac::HMAC_SHA256, &self.key),
            token.as_bytes(),
            &expected,
        )
        .map_err(|_| anyhow!("invalid research capability"))?;
        if grant.session_id.is_none() && !allow_pending {
            bail!("research grant is not bound");
        }
        Ok(ResearchScope {
            project_id: grant.project_id.clone(),
            experiment_id: grant.experiment_id.clone(),
            session_id: grant.session_id.clone(),
            grant_id: grant_id.to_owned(),
        })
    }

    pub(crate) fn session_scopes(&self, session_id: &str) -> Vec<ResearchScope> {
        let Ok(state) = self.state.lock() else {
            return vec![];
        };
        let now = now().unwrap_or(u64::MAX);
        state
            .grants
            .iter()
            .filter_map(|(id, grant)| {
                (grant.session_id.as_deref() == Some(session_id)
                    && !grant.revoked
                    && grant.expires_at > now)
                    .then(|| ResearchScope {
                        project_id: grant.project_id.clone(),
                        experiment_id: grant.experiment_id.clone(),
                        session_id: grant.session_id.clone(),
                        grant_id: id.clone(),
                    })
            })
            .collect()
    }

    pub(crate) fn resolver(&self) -> crate::keeper::McpServerResolver {
        let this = self.clone();
        Arc::new(move |session| {
            let Ok(state) = this.state.lock() else {
                return vec![];
            };
            let now = now().unwrap_or(u64::MAX);
            state
                .grants
                .iter()
                .filter_map(|(id, grant)| {
                    (grant.session_id.as_deref() == Some(session)
                        && !grant.revoked
                        && grant.expires_at > now)
                        .then(|| this.server_config(id, &this.token(id)))
                })
                .collect()
        })
    }

    pub(crate) async fn revoke(&self, grant_id: &str) -> Result<()> {
        validate_id(grant_id)?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow!("research grants lock poisoned"))?;
        let grant = state
            .grants
            .get_mut(grant_id)
            .ok_or_else(|| anyhow!("unknown research grant"))?;
        grant.revoked = true;
        self.persist(&state)
    }

    fn server_config(&self, id: &str, token: &str) -> Value {
        let exe = &*self.mcp_exe;
        let mut args = vec![json!("research-mcp")];
        match &self.endpoint {
            NodeEndpoint::Http(url) => args.extend([json!("--node-url"), json!(url)]),
            NodeEndpoint::Unix(path) => args.extend([json!("--node-socket"), json!(path)]),
        }
        args.extend([json!("--grant-id"), json!(id)]);
        json!({"name":"postudio-research","command":exe,"args":args,"env":[{"name":"POSTUDIO_RESEARCH_TOKEN","value":token}]})
    }
    fn token(&self, id: &str) -> String {
        URL_SAFE_NO_PAD.encode(
            hmac::sign(
                &hmac::Key::new(hmac::HMAC_SHA256, &self.key),
                format!("postudio-research-grant:{id}").as_bytes(),
            )
            .as_ref(),
        )
    }
    fn capability_hash(&self, token: &str) -> String {
        URL_SAFE_NO_PAD.encode(
            hmac::sign(
                &hmac::Key::new(hmac::HMAC_SHA256, &self.key),
                token.as_bytes(),
            )
            .as_ref(),
        )
    }
    fn persist(&self, store: &Store) -> Result<()> {
        let bytes = serde_json::to_vec(store)?;
        let parent = self
            .path
            .parent()
            .ok_or_else(|| anyhow!("grant store has no parent"))?;
        let temp = parent.join(format!(
            "grants.{}.{}.{}.tmp",
            std::process::id(),
            now()?,
            TEMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        {
            use std::io::Write;
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut f = options.open(&temp)?;
            f.write_all(&bytes)?;
            f.sync_all()?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&temp, fs::Permissions::from_mode(0o600))?;
        }
        let result = fs::rename(&temp, &*self.path);
        if result.is_err() {
            let _ = fs::remove_file(temp);
        }
        result.context("commit research grant store")
    }
}

fn validate_id(id: &str) -> Result<()> {
    if id.is_empty()
        || id.len() > 256
        || id.chars().any(|c| c.is_control() || c == '/' || c == '\\')
    {
        bail!("invalid research identifier")
    }
    Ok(())
}
fn now() -> Result<u64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs())
}
fn ensure_live(g: &Grant) -> Result<()> {
    if g.revoked || g.expires_at <= now()? {
        bail!("research grant expired or revoked");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn failed_binding_stays_pending_until_durable_retry() {
        let dir = tempfile::tempdir().unwrap();
        let bindings = ResearchBindings::open(
            dir.path(),
            NodeEndpoint::Http("http://127.0.0.1:7440".into()),
        )
        .unwrap();
        let grant = bindings
            .create_pending("project", "experiment")
            .await
            .unwrap();
        let token = bindings.token(&grant.id);
        let saved = bindings.path.with_extension("saved");
        std::fs::rename(&*bindings.path, &saved).unwrap();
        std::fs::create_dir(&*bindings.path).unwrap();
        assert!(bindings.bind(&grant.id, "session").await.is_err());
        assert!(bindings.authorize(&grant.id, &token, false).await.is_err());
        std::fs::remove_dir(&*bindings.path).unwrap();
        std::fs::rename(saved, &*bindings.path).unwrap();
        bindings.bind(&grant.id, "session").await.unwrap();
        assert!(bindings.authorize(&grant.id, &token, false).await.is_ok());
    }
    #[tokio::test]
    async fn persists_derivation_and_binds_once() -> Result<()> {
        let dir = tempdir()?;
        let endpoint = NodeEndpoint::Http("http://127.0.0.1:7410".into());
        let bindings = ResearchBindings::open(dir.path(), endpoint.clone())?;
        let pending = bindings.create_pending("project", "experiment").await?;
        let token = pending.mcp_server["env"][0]["value"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(
            bindings
                .authorize(&pending.id, &token, false)
                .await
                .is_err()
        );
        bindings.authorize(&pending.id, &token, true).await?;
        bindings.bind(&pending.id, "session").await?;
        bindings.bind(&pending.id, "session").await?;
        assert!(bindings.bind(&pending.id, "other").await.is_err());
        let reopened = ResearchBindings::open(dir.path(), endpoint)?;
        assert_eq!(
            reopened
                .authorize(&pending.id, &token, false)
                .await?
                .session_id
                .as_deref(),
            Some("session")
        );
        assert!(
            reopened
                .authorize(&pending.id, "wrong", false)
                .await
                .is_err()
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(dir.path().join("research-sessions/hmac.key"))?
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn revoke_and_cross_scope_are_denied() -> Result<()> {
        let dir = tempdir()?;
        let b = ResearchBindings::open(dir.path(), NodeEndpoint::Unix("/tmp/node.sock".into()))?;
        let g = b.create_pending("p", "e").await?;
        let token = g.mcp_server["env"][0]["value"].as_str().unwrap();
        b.bind(&g.id, "s").await?;
        assert_eq!(b.authorize(&g.id, token, false).await?.project_id, "p");
        b.revoke(&g.id).await?;
        assert!(b.authorize(&g.id, token, true).await.is_err());
        Ok(())
    }
}
