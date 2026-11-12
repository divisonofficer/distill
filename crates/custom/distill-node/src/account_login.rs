//! Browser login jobs. Authorization URLs use a loopback callback on the node, so browser must run on same computer.

use std::process::Stdio;
use std::sync::{Arc, Weak};
use std::time::Duration;

use axum::Json;
use axum::Router;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ring::rand::{SecureRandom as _, SystemRandom};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt as _};
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, oneshot};
use tokio::time::timeout;
use url::Url;

use crate::TAILSCALE_USER_LOGIN;
use crate::agent::AgentConfig;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(300);
const DEFAULT_TTL: Duration = Duration::from_secs(600);
const DEFAULT_MAX_JOBS: usize = 16;
const DEFAULT_MAX_OUTPUT: usize = 64 * 1024;
const DEFAULT_MAX_LINE: usize = 8 * 1024;
const ID_RANDOM_BYTES: usize = 16;
const AUTHORIZATION_HOSTS: [&str; 3] = ["auth.openai.com", "claude.ai", "openrouter.ai"];

/// Short limits support deterministic tests; production uses 300 seconds, 10 minutes, 16 jobs and bounded output.
#[derive(Clone, Copy, Debug)]
pub struct LoginLimits {
    pub timeout: Duration,
    pub ttl: Duration,
    pub max_jobs: usize,
    pub max_output_bytes: usize,
    pub max_line_bytes: usize,
}

impl Default for LoginLimits {
    fn default() -> Self {
        Self {
            timeout: DEFAULT_TIMEOUT,
            ttl: DEFAULT_TTL,
            max_jobs: DEFAULT_MAX_JOBS,
            max_output_bytes: DEFAULT_MAX_OUTPUT,
            max_line_bytes: DEFAULT_MAX_LINE,
        }
    }
}

#[derive(Clone)]
struct RouteState {
    manager: Arc<Manager>,
    cfg: AgentConfig,
    limits: LoginLimits,
}

struct Manager {
    jobs: Mutex<Jobs>,
}

struct Jobs {
    jobs: Vec<Job>,
    active_id: Option<String>,
}

struct Job {
    id: String,
    owner: String,
    status: JobStatus,
    authorization_url: Option<String>,
    terminal_at: Option<tokio::time::Instant>,
    cancel: Option<oneshot::Sender<()>>,
    ttl: Duration,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum JobStatus {
    Starting,
    Pending,
    Completed,
    Failed,
    Cancelled,
    Expired,
}

impl JobStatus {
    fn terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Cancelled | Self::Expired
        )
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum CallbackMode {
    HostLoopback,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct JobView {
    job_id: String,
    status: JobStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    authorization_url: Option<String>,
    callback_mode: CallbackMode,
}

impl JobView {
    fn from_job(job: &Job) -> Self {
        Self {
            job_id: job.id.clone(),
            status: job.status,
            authorization_url: job.authorization_url.clone(),
            callback_mode: CallbackMode::HostLoopback,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StartRequest {
    id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CancelRequest {}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: &'static str,
}

impl ApiError {
    fn new(status: StatusCode, message: &'static str) -> Self {
        Self { status, message }
    }

    fn invalid_id() -> Self {
        Self::new(StatusCode::BAD_REQUEST, "Invalid account id.")
    }

    fn not_found() -> Self {
        Self::new(StatusCode::NOT_FOUND, "Login job not found.")
    }

    fn forbidden() -> Self {
        Self::new(StatusCode::FORBIDDEN, "Node owner required.")
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

/// Mount login job endpoints. Browser authorization and callback stay on node localhost; no callback or token is forwarded.
pub fn routes(cfg: AgentConfig) -> Router {
    routes_inner(cfg, LoginLimits::default())
}

/// Mount login endpoints with shorter limits for tests.
#[doc(hidden)]
pub fn routes_with_limits(cfg: AgentConfig, limits: LoginLimits) -> Router {
    routes_inner(cfg, limits)
}

fn routes_inner(cfg: AgentConfig, limits: LoginLimits) -> Router {
    let state = RouteState {
        manager: Arc::new(Manager {
            jobs: Mutex::new(Jobs {
                jobs: Vec::new(),
                active_id: None,
            }),
        }),
        cfg,
        limits: LoginLimits {
            max_jobs: limits.max_jobs.max(1),
            max_line_bytes: limits.max_line_bytes.max(1),
            max_output_bytes: limits.max_output_bytes.max(1),
            ..limits
        },
    };
    Router::new()
        .route("/api/accounts/login", post(start))
        .route("/api/accounts/login/{job_id}", get(poll))
        .route("/api/accounts/login/{job_id}/cancel", post(cancel))
        .with_state(state)
}

async fn start(
    State(state): State<RouteState>,
    headers: HeaderMap,
    Json(request): Json<StartRequest>,
) -> Result<impl IntoResponse, ApiError> {
    validate_id(&request.id)?;
    let owner = owner(&headers)?;
    let (job_id, cancel_rx) = {
        let mut jobs = state.manager.jobs.lock().await;
        prune_expired(&mut jobs.jobs);
        if jobs.active_id.is_some() {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "A login job is already active.",
            ));
        }
        while jobs.jobs.len() >= state.limits.max_jobs {
            jobs.jobs.remove(0);
        }
        let id = new_job_id(&jobs.jobs).ok_or_else(|| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Could not start login job.",
            )
        })?;
        let (cancel_tx, cancel_rx) = oneshot::channel();
        jobs.jobs.push(Job {
            id: id.clone(),
            owner,
            status: JobStatus::Starting,
            authorization_url: None,
            terminal_at: None,
            cancel: Some(cancel_tx),
            ttl: state.limits.ttl,
        });
        jobs.active_id = Some(id.clone());
        (id, cancel_rx)
    };

    tokio::spawn(run_job(
        Arc::downgrade(&state.manager),
        job_id.clone(),
        request.id,
        state.cfg,
        state.limits,
        cancel_rx,
    ));
    let response = JobView {
        job_id,
        status: JobStatus::Starting,
        authorization_url: None,
        callback_mode: CallbackMode::HostLoopback,
    };
    Ok((StatusCode::ACCEPTED, Json(response)))
}

async fn poll(
    State(state): State<RouteState>,
    headers: HeaderMap,
    Path(job_id): Path<String>,
) -> Result<Json<JobView>, ApiError> {
    let owner = owner(&headers)?;
    let mut jobs = state.manager.jobs.lock().await;
    prune_expired(&mut jobs.jobs);
    let job = jobs
        .jobs
        .iter()
        .find(|job| job.id == job_id && job.owner == owner)
        .ok_or_else(ApiError::not_found)?;
    Ok(Json(JobView::from_job(job)))
}

async fn cancel(
    State(state): State<RouteState>,
    headers: HeaderMap,
    Path(job_id): Path<String>,
    Json(_request): Json<CancelRequest>,
) -> Result<Json<JobView>, ApiError> {
    let owner = owner(&headers)?;
    let cancel = {
        let mut jobs = state.manager.jobs.lock().await;
        prune_expired(&mut jobs.jobs);
        let job = jobs
            .jobs
            .iter_mut()
            .find(|job| job.id == job_id && job.owner == owner)
            .ok_or_else(ApiError::not_found)?;
        if !job.status.terminal() {
            job.status = JobStatus::Cancelled;
            job.authorization_url = None;
            job.cancel.take()
        } else {
            None
        }
    };
    if let Some(cancel) = cancel {
        let _ = cancel.send(());
    }
    let jobs = state.manager.jobs.lock().await;
    let job = jobs
        .jobs
        .iter()
        .find(|job| job.id == job_id && job.owner == owner)
        .ok_or_else(ApiError::not_found)?;
    Ok(Json(JobView::from_job(job)))
}

fn owner(headers: &HeaderMap) -> Result<String, ApiError> {
    headers
        .get(TAILSCALE_USER_LOGIN)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(ApiError::forbidden)
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
        return Err(ApiError::invalid_id());
    }
    Ok(())
}

fn prune_expired(jobs: &mut [Job]) {
    let now = tokio::time::Instant::now();
    for job in jobs {
        if job.status.terminal()
            && job.status != JobStatus::Expired
            && job
                .terminal_at
                .is_some_and(|at| now.duration_since(at) >= job.ttl)
        {
            job.status = JobStatus::Expired;
            job.authorization_url = None;
        }
    }
}

fn new_job_id(jobs: &[Job]) -> Option<String> {
    let rng = SystemRandom::new();
    for _ in 0..8 {
        let mut bytes = [0; ID_RANDOM_BYTES];
        rng.fill(&mut bytes).ok()?;
        let id = URL_SAFE_NO_PAD.encode(bytes);
        if jobs.iter().all(|job| job.id != id) {
            return Some(id);
        }
    }
    None
}

#[derive(Debug, Deserialize)]
#[serde(tag = "status", deny_unknown_fields)]
enum CliEvent {
    #[serde(rename = "pending")]
    Pending {
        #[serde(rename = "authorizationUrl")]
        authorization_url: String,
        #[serde(rename = "callbackMode")]
        callback_mode: CallbackMode,
    },
    #[serde(rename = "completed")]
    Completed {},
    #[serde(rename = "failed")]
    Failed {},
}

#[derive(Debug, Default)]
struct EventsSeen {
    pending: bool,
    completed: bool,
}

#[derive(Clone, Copy, Debug)]
enum ProcessError {
    InvalidOutput,
    OutputLimit,
    LineLimit,
    Io,
}

async fn run_job(
    manager: Weak<Manager>,
    job_id: String,
    account_id: String,
    cfg: AgentConfig,
    limits: LoginLimits,
    cancel_rx: oneshot::Receiver<()>,
) {
    let mut command = Command::new(&cfg.distill_bin);
    command
        .args(["accounts", "login-web", &account_id])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    for (key, value) in &cfg.extra_env {
        command.env(key, value);
    }
    #[allow(
        clippy::disallowed_methods,
        reason = "login command uses fixed arguments, bounded output, and kill-on-drop"
    )]
    let child = command.spawn();
    let status = match child {
        Ok(child) => run_child(child, manager.clone(), &job_id, limits, cancel_rx).await,
        Err(_) => JobStatus::Failed,
    };
    finish_job(&manager, &job_id, status).await;
}

async fn run_child(
    mut child: Child,
    manager: Weak<Manager>,
    id: &str,
    limits: LoginLimits,
    mut cancel_rx: oneshot::Receiver<()>,
) -> JobStatus {
    let Some(stdout) = child.stdout.take() else {
        reap(&mut child).await;
        return JobStatus::Failed;
    };
    let Some(stderr) = child.stderr.take() else {
        reap(&mut child).await;
        return JobStatus::Failed;
    };

    enum Outcome {
        Finished(Result<(std::process::ExitStatus, EventsSeen), ProcessError>),
        Cancelled,
        TimedOut,
    }
    let outcome = {
        let process = async {
            let (exit, events, ()) = tokio::try_join!(
                async { child.wait().await.map_err(|_| ProcessError::Io) },
                read_events(stdout, manager.clone(), id.to_owned(), limits),
                discard(stderr),
            )?;
            Ok::<_, ProcessError>((exit, events))
        };
        tokio::select! {
            _ = &mut cancel_rx => Outcome::Cancelled,
            result = timeout(limits.timeout, process) => match result {
                Ok(result) => Outcome::Finished(result),
                Err(_) => Outcome::TimedOut,
            },
        }
    };

    match outcome {
        Outcome::Finished(Ok((exit, events))) => {
            if exit.success() && events.completed {
                JobStatus::Completed
            } else {
                JobStatus::Failed
            }
        }
        Outcome::Cancelled => {
            reap(&mut child).await;
            JobStatus::Cancelled
        }
        Outcome::Finished(Err(_)) | Outcome::TimedOut => {
            reap(&mut child).await;
            JobStatus::Failed
        }
    }
}

async fn reap(child: &mut Child) {
    #[cfg(unix)]
    if let Some(pid) = child.id() {
        // Let the CLI drop its staged credentials before escalating to SIGKILL.
        #[allow(clippy::disallowed_methods, reason = "fixed signal command for owned login child")]
        let signal = Command::new("/bin/kill")
            .args(["-TERM", &pid.to_string()])
            .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
            .kill_on_drop(true).status();
        let _ = timeout(Duration::from_secs(1), signal).await;
        if timeout(Duration::from_secs(2), child.wait()).await.is_ok() {
            return;
        }
    }
    let _ = child.kill().await;
    let _ = child.wait().await;
}

async fn discard<R: AsyncRead + Unpin>(mut reader: R) -> Result<(), ProcessError> {
    let mut sink = tokio::io::sink();
    tokio::io::copy(&mut reader, &mut sink)
        .await
        .map(|_| ())
        .map_err(|_| ProcessError::Io)
}

async fn read_events<R: AsyncRead + Unpin>(
    mut reader: R,
    manager: Weak<Manager>,
    id: String,
    limits: LoginLimits,
) -> Result<EventsSeen, ProcessError> {
    let mut total = 0usize;
    let mut line = Vec::with_capacity(1024);
    let mut chunk = [0u8; 4096];
    let mut events = EventsSeen::default();
    loop {
        let count = reader
            .read(&mut chunk)
            .await
            .map_err(|_| ProcessError::Io)?;
        if count == 0 {
            if !line.is_empty() {
                consume_line(&line, &mut events, &manager, &id).await?;
            }
            return Ok(events);
        }
        total = total.checked_add(count).ok_or(ProcessError::OutputLimit)?;
        if total > limits.max_output_bytes {
            return Err(ProcessError::OutputLimit);
        }
        for byte in &chunk[..count] {
            line.push(*byte);
            if line.len() > limits.max_line_bytes {
                return Err(ProcessError::LineLimit);
            }
            if *byte == b'\n' {
                consume_line(&line, &mut events, &manager, &id).await?;
                line.clear();
            }
        }
    }
}

async fn consume_line(
    bytes: &[u8],
    events: &mut EventsSeen,
    manager: &Weak<Manager>,
    id: &str,
) -> Result<(), ProcessError> {
    let event: CliEvent = serde_json::from_slice(bytes).map_err(|_| ProcessError::InvalidOutput)?;
    match event {
        CliEvent::Pending {
            authorization_url,
            callback_mode: CallbackMode::HostLoopback,
        } if !events.pending && !events.completed => {
            validate_authorization_url(&authorization_url)?;
            let manager = manager.upgrade().ok_or(ProcessError::InvalidOutput)?;
            let mut jobs = manager.jobs.lock().await;
            let job = jobs
                .jobs
                .iter_mut()
                .find(|job| job.id == id)
                .ok_or(ProcessError::InvalidOutput)?;
            if job.status != JobStatus::Starting {
                return Err(ProcessError::InvalidOutput);
            }
            job.status = JobStatus::Pending;
            job.authorization_url = Some(authorization_url);
            events.pending = true;
        }
        CliEvent::Completed {} if events.pending && !events.completed => {
            events.completed = true;
        }
        CliEvent::Failed {} => return Err(ProcessError::InvalidOutput),
        _ => return Err(ProcessError::InvalidOutput),
    }
    Ok(())
}

fn validate_authorization_url(value: &str) -> Result<(), ProcessError> {
    let url = Url::parse(value).map_err(|_| ProcessError::InvalidOutput)?;
    let authority = url
        .as_str()
        .split_once("://")
        .map(|(_, rest)| rest.split(['/', '?', '#']).next().unwrap_or(rest))
        .unwrap_or_default();
    if url.scheme() != "https"
        || authority.contains('@')
        || url.username() != ""
        || url.password().is_some()
        || url.port().is_some()
        || url.fragment().is_some()
        || !url
            .host_str()
            .is_some_and(|host| AUTHORIZATION_HOSTS.contains(&host))
    {
        return Err(ProcessError::InvalidOutput);
    }
    Ok(())
}

async fn finish_job(manager: &Weak<Manager>, id: &str, status: JobStatus) {
    let Some(manager) = manager.upgrade() else {
        return;
    };
    let mut jobs = manager.jobs.lock().await;
    if let Some(job) = jobs.jobs.iter_mut().find(|job| job.id == id) {
        if job.status != JobStatus::Cancelled {
            job.status = status;
        }
        job.authorization_url = None;
        job.cancel.take();
        job.terminal_at = Some(tokio::time::Instant::now());
    }
    if jobs.active_id.as_deref() == Some(id) {
        jobs.active_id = None;
    }
}
