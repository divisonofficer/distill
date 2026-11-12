use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncReadExt as _};
use tokio::process::Command;
use tokio::time::timeout;

use crate::agent::AgentConfig;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_COMMAND_OUTPUT: usize = 1024 * 1024;
static ROUTING_TEMP_SEQUENCE: AtomicUsize = AtomicUsize::new(0);

pub fn routes(cfg: AgentConfig) -> Router {
    Router::new()
        .route("/api/accounts", get(list))
        .route("/api/accounts/add", post(add))
        .route("/api/accounts/rename", post(rename))
        .route("/api/accounts/logout", post(logout))
        .route("/api/accounts/remove", post(remove))
        .route("/api/accounts/usage", post(usage))
        .route("/api/accounts/routing", get(routing_get).post(routing_set))
        .with_state(cfg)
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Provider {
    Chatgpt,
    Claude,
    Grok,
    Openrouter,
}

impl Provider {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "chatgpt" => Some(Self::Chatgpt),
            "claude" => Some(Self::Claude),
            "grok" => Some(Self::Grok),
            "openrouter" => Some(Self::Openrouter),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Chatgpt => "chatgpt",
            Self::Claude => "claude",
            Self::Grok => "grok",
            Self::Openrouter => "openrouter",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum ConnectionError {
    Unreadable,
    ConfigurationRequired,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum QuotaStatus {
    Available,
    Unavailable,
    Unsupported,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct QuotaView {
    account_id: String,
    checked_at: String,
    fetched_at: Option<String>,
    status: QuotaStatus,
    remaining_percent: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct CliAccountStatus {
    id: String,
    provider: Provider,
    label: String,
    legacy: bool,
    connected: bool,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    connection_error: Option<ConnectionError>,
    #[serde(default)]
    quota: Option<QuotaView>,
}

#[derive(Debug, Serialize)]
struct AccountView {
    id: String,
    provider: Provider,
    label: String,
    legacy: bool,
    connected: bool,
    email: Option<String>,
    connection_error: Option<ConnectionError>,
    quota: Option<QuotaView>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AddRequest {
    id: String,
    provider: String,
    label: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RenameRequest {
    id: String,
    label: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct IdRequest {
    id: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum RoutingStrategy {
    OrderedFirst,
    HighestRemaining,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct AutoRouting {
    enabled: bool,
    strategy: RoutingStrategy,
    options: Vec<AutoOption>,
}

#[derive(Debug, Deserialize)]
struct CliRouting {
    enabled: bool,
    strategy: RoutingStrategy,
    options: Vec<CliAutoOption>,
}

#[derive(Debug, Deserialize)]
struct CliAutoOption {
    id: String,
    account_id: String,
    main_model: String,
    #[serde(default)]
    worker_model: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct AutoOption {
    id: String,
    account_id: String,
    main_model: String,
    #[serde(default)]
    worker_model: Option<String>,
}

#[derive(Debug, Serialize)]
struct Success {
    ok: bool,
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: &'static str,
}

impl ApiError {
    fn invalid(message: &'static str) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message,
        }
    }

    fn failed() -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
            message: "Account command failed.",
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(serde_json::json!({ "error": self.message })),
        )
            .into_response()
    }
}

async fn list(State(cfg): State<AgentConfig>) -> Result<Json<Vec<AccountView>>, ApiError> {
    let output = run_cli(&cfg, &[OsStr::new("accounts"), OsStr::new("list")]).await?;
    let accounts: Vec<CliAccountStatus> =
        serde_json::from_slice(&output).map_err(|_| ApiError::failed())?;
    Ok(Json(
        accounts
            .into_iter()
            .map(|account| AccountView {
                id: account.id,
                provider: account.provider,
                label: account.label,
                legacy: account.legacy,
                connected: account.connected,
                email: account.email,
                connection_error: account.connection_error,
                quota: sanitize_quota(account.quota),
            })
            .collect(),
    ))
}

async fn add(
    State(cfg): State<AgentConfig>,
    Json(request): Json<AddRequest>,
) -> Result<Json<Success>, ApiError> {
    validate_id(&request.id)?;
    validate_label(&request.label)?;
    let provider = Provider::parse(&request.provider)
        .ok_or_else(|| ApiError::invalid("Unsupported account provider."))?;
    mutate(
        &cfg,
        &[
            OsStr::new("accounts"),
            OsStr::new("add"),
            OsStr::new(&request.id),
            OsStr::new("--provider"),
            OsStr::new(provider.as_str()),
            OsStr::new("--label"),
            OsStr::new(&request.label),
        ],
    )
    .await
}

async fn rename(
    State(cfg): State<AgentConfig>,
    Json(request): Json<RenameRequest>,
) -> Result<Json<Success>, ApiError> {
    validate_id(&request.id)?;
    validate_label(&request.label)?;
    mutate(
        &cfg,
        &[
            OsStr::new("accounts"),
            OsStr::new("rename"),
            OsStr::new(&request.id),
            OsStr::new("--label"),
            OsStr::new(&request.label),
        ],
    )
    .await
}

async fn logout(
    State(cfg): State<AgentConfig>,
    Json(request): Json<IdRequest>,
) -> Result<Json<Success>, ApiError> {
    validate_id(&request.id)?;
    mutate(
        &cfg,
        &[
            OsStr::new("accounts"),
            OsStr::new("logout"),
            OsStr::new(&request.id),
        ],
    )
    .await
}

async fn remove(
    State(cfg): State<AgentConfig>,
    Json(request): Json<IdRequest>,
) -> Result<Json<Success>, ApiError> {
    validate_id(&request.id)?;
    mutate(
        &cfg,
        &[
            OsStr::new("accounts"),
            OsStr::new("remove"),
            OsStr::new(&request.id),
        ],
    )
    .await
}

async fn usage(
    State(cfg): State<AgentConfig>,
    Json(request): Json<IdRequest>,
) -> Result<Json<QuotaView>, ApiError> {
    validate_id(&request.id)?;
    let output = run_cli(
        &cfg,
        &[
            OsStr::new("accounts"),
            OsStr::new("usage"),
            OsStr::new(&request.id),
        ],
    )
    .await?;
    let quota: QuotaView = serde_json::from_slice(&output).map_err(|_| ApiError::failed())?;
    Ok(Json(
        sanitize_quota(Some(quota)).ok_or_else(ApiError::failed)?,
    ))
}

async fn routing_get(State(cfg): State<AgentConfig>) -> Result<Json<AutoRouting>, ApiError> {
    Ok(Json(load_routing(&cfg).await?))
}

async fn routing_set(
    State(cfg): State<AgentConfig>,
    Json(routing): Json<AutoRouting>,
) -> Result<Json<AutoRouting>, ApiError> {
    validate_routing(&routing)?;
    let temp_file = RoutingTempFile::create(&routing)?;
    let output = run_cli(
        &cfg,
        &[
            OsStr::new("accounts"),
            OsStr::new("routing"),
            OsStr::new("set"),
            OsStr::new("--file"),
            temp_file.path.as_os_str(),
        ],
    )
    .await?;
    let _: Value = serde_json::from_slice(&output).map_err(|_| ApiError::failed())?;
    drop(temp_file);
    Ok(Json(load_routing(&cfg).await?))
}

async fn load_routing(cfg: &AgentConfig) -> Result<AutoRouting, ApiError> {
    let output = run_cli(
        cfg,
        &[
            OsStr::new("accounts"),
            OsStr::new("routing"),
            OsStr::new("get"),
        ],
    )
    .await?;
    let cli_routing: CliRouting =
        serde_json::from_slice(&output).map_err(|_| ApiError::failed())?;
    let routing = AutoRouting {
        enabled: cli_routing.enabled,
        strategy: cli_routing.strategy,
        options: cli_routing
            .options
            .into_iter()
            .map(|option| AutoOption {
                id: option.id,
                account_id: option.account_id,
                main_model: option.main_model,
                worker_model: option.worker_model,
            })
            .collect(),
    };
    validate_routing(&routing).map_err(|_| ApiError::failed())?;
    Ok(routing)
}

fn validate_routing(routing: &AutoRouting) -> Result<(), ApiError> {
    if routing.options.len() > 64 || (routing.enabled && routing.options.is_empty()) {
        return Err(ApiError::invalid("Invalid account routing configuration."));
    }
    let mut ids = std::collections::HashSet::new();
    for option in &routing.options {
        if !valid_routing_id(&option.id) || !ids.insert(&option.id) {
            return Err(ApiError::invalid("Invalid routing option id."));
        }
        if !valid_routing_id(&option.account_id) {
            return Err(ApiError::invalid("Invalid account id."));
        }
        if !valid_model_name(&option.main_model)
            || option
                .worker_model
                .as_deref()
                .is_some_and(|model| !valid_model_name(model))
        {
            return Err(ApiError::invalid("Invalid routing model."));
        }
    }
    Ok(())
}

fn valid_routing_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 64
        && (bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit())
        && bytes.iter().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'_' || *byte == b'-'
        })
}

fn valid_model_name(model: &str) -> bool {
    !model.trim().is_empty() && model.len() <= 256
}

struct RoutingTempFile {
    path: PathBuf,
}

impl RoutingTempFile {
    fn create(routing: &AutoRouting) -> Result<Self, ApiError> {
        let contents = serde_json::to_vec(routing).map_err(|_| ApiError::failed())?;
        let directory = std::env::temp_dir();
        for _ in 0..16 {
            let sequence = ROUTING_TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = directory.join(format!(
                ".distill-routing-{}-{sequence}.json",
                std::process::id()
            ));
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = match options.open(&path) {
                Ok(file) => file,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(_) => return Err(ApiError::failed()),
            };
            let temp_file = Self { path };
            file.write_all(&contents).map_err(|_| ApiError::failed())?;
            drop(file);
            return Ok(temp_file);
        }
        Err(ApiError::failed())
    }
}

impl Drop for RoutingTempFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

async fn mutate(cfg: &AgentConfig, args: &[&OsStr]) -> Result<Json<Success>, ApiError> {
    let output = run_cli(cfg, args).await?;
    let _: Value = serde_json::from_slice(&output).map_err(|_| ApiError::failed())?;
    Ok(Json(Success { ok: true }))
}

fn validate_id(id: &str) -> Result<(), ApiError> {
    let bytes = id.as_bytes();
    if bytes.is_empty()
        || bytes.len() > 64
        || bytes[0] == b'-'
        || !bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_' || *byte == b'-')
    {
        return Err(ApiError::invalid("Invalid account id."));
    }
    Ok(())
}

fn validate_label(label: &str) -> Result<(), ApiError> {
    if label.trim().is_empty() || label.chars().count() > 120 {
        return Err(ApiError::invalid("Invalid account label."));
    }
    Ok(())
}

fn sanitize_quota(quota: Option<QuotaView>) -> Option<QuotaView> {
    quota.map(|mut quota| {
        if quota
            .remaining_percent
            .is_some_and(|percent| !percent.is_finite() || !(0.0..=100.0).contains(&percent))
        {
            quota.remaining_percent = None;
        }
        quota
    })
}

async fn run_cli(cfg: &AgentConfig, args: &[&OsStr]) -> Result<Vec<u8>, ApiError> {
    let mut command = Command::new(&cfg.distill_bin);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    for (key, value) in &cfg.extra_env {
        command.env(key, value);
    }
    #[allow(
        clippy::disallowed_methods,
        reason = "account command uses fixed arguments and bounded output"
    )]
    let mut child = command.spawn().map_err(|_| ApiError::failed())?;
    let stdout = child.stdout.take().ok_or_else(ApiError::failed)?;
    let stderr = child.stderr.take().ok_or_else(ApiError::failed)?;
    let output_size = Arc::new(AtomicUsize::new(0));

    let result = timeout(COMMAND_TIMEOUT, async {
        tokio::try_join!(
            child.wait(),
            read_bounded(stdout, output_size.clone()),
            read_bounded(stderr, output_size),
        )
    })
    .await;

    match result {
        Ok(Ok((status, output, _stderr))) if status.success() => Ok(output),
        _ => {
            if child.id().is_some() {
                let _ = child.start_kill();
                let _ = child.wait().await;
            }
            Err(ApiError::failed())
        }
    }
}

async fn read_bounded<R>(mut reader: R, total: Arc<AtomicUsize>) -> io::Result<Vec<u8>>
where
    R: AsyncRead + Unpin,
{
    let mut output = Vec::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let read = reader.read(&mut buffer).await?;
        if read == 0 {
            return Ok(output);
        }
        let reservation = total.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |used| {
            used.checked_add(read)
                .filter(|size| *size <= MAX_COMMAND_OUTPUT)
        });
        if reservation.is_err() {
            return Err(io::Error::other("command output limit exceeded"));
        }
        output.extend_from_slice(&buffer[..read]);
    }
}
