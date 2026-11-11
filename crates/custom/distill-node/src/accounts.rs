use std::io;
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

pub fn routes(cfg: AgentConfig) -> Router {
    Router::new()
        .route("/api/accounts", get(list))
        .route("/api/accounts/add", post(add))
        .route("/api/accounts/rename", post(rename))
        .route("/api/accounts/logout", post(logout))
        .route("/api/accounts/remove", post(remove))
        .route("/api/accounts/usage", post(usage))
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
    let output = run_cli(&cfg, &["accounts", "list"]).await?;
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
            "accounts",
            "add",
            &request.id,
            "--provider",
            provider.as_str(),
            "--label",
            &request.label,
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
        &["accounts", "rename", &request.id, "--label", &request.label],
    )
    .await
}

async fn logout(
    State(cfg): State<AgentConfig>,
    Json(request): Json<IdRequest>,
) -> Result<Json<Success>, ApiError> {
    validate_id(&request.id)?;
    mutate(&cfg, &["accounts", "logout", &request.id]).await
}

async fn remove(
    State(cfg): State<AgentConfig>,
    Json(request): Json<IdRequest>,
) -> Result<Json<Success>, ApiError> {
    validate_id(&request.id)?;
    mutate(&cfg, &["accounts", "remove", &request.id]).await
}

async fn usage(
    State(cfg): State<AgentConfig>,
    Json(request): Json<IdRequest>,
) -> Result<Json<QuotaView>, ApiError> {
    validate_id(&request.id)?;
    let output = run_cli(&cfg, &["accounts", "usage", &request.id]).await?;
    let quota: QuotaView = serde_json::from_slice(&output).map_err(|_| ApiError::failed())?;
    Ok(Json(
        sanitize_quota(Some(quota)).ok_or_else(ApiError::failed)?,
    ))
}

async fn mutate(cfg: &AgentConfig, args: &[&str]) -> Result<Json<Success>, ApiError> {
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

async fn run_cli(cfg: &AgentConfig, args: &[&str]) -> Result<Vec<u8>, ApiError> {
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
