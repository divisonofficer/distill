use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};

use axum::{
    Json, Router,
    extract::{Path as AxumPath, Query, State},
    http::{Method, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use ring::{digest, hmac, rand::SecureRandom};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::{
    process::{Child, Command},
    sync::Mutex,
};

const EXPECTED_VERSION: &str = "0.2.27";
const BODY_LIMIT: usize = 1024 * 1024;

#[derive(Clone)]
pub(crate) struct ResearchState {
    bin: Option<PathBuf>,
    root: PathBuf,
    child: Arc<Mutex<Option<Child>>>,
    run_submission: Arc<Mutex<()>>,
    client: reqwest::Client,
    blocked_ports: crate::ports::PortPolicy,
    refs: crate::research_refs::RefsStore,
    papers: crate::research_library::PaperCache,
}
pub(crate) type ResearchService = ResearchState;

/// Why a read-only `orx` call produced no usable output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OrxCallError {
    NotConfigured,
    Invalid,
    Launch,
    Read,
    TooLarge,
    Failed,
    Timeout,
}

impl OrxCallError {
    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::NotConfigured => "OpenResearch not configured",
            Self::Invalid => "Invalid arguments",
            Self::Launch => "Could not launch OpenResearch",
            Self::Read => "Could not read OpenResearch output",
            Self::TooLarge => "OpenResearch output too large",
            Self::Failed => "OpenResearch provider unavailable or disabled",
            Self::Timeout => "OpenResearch provider timed out",
        }
    }
}
#[derive(Clone, Deserialize)]
struct Descriptor {
    #[serde(default)]
    pid: u32,
    instance: String,
    port: u16,
    version: String,
    proto: u32,
    token: String,
    #[serde(default, rename = "privatePorts")]
    private_ports: Vec<u16>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Status {
    available: bool,
    connected: bool,
    version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}
#[derive(Deserialize)]
struct ProjectReq {
    name: String,
    path: String,
}
#[derive(Deserialize)]
struct ExperimentReq {
    title: String,
    description: Option<String>,
    #[serde(rename = "runCommand")]
    run_command: Option<String>,
    #[serde(rename = "parentId")]
    parent_id: Option<String>,
}
#[derive(Deserialize)]
struct Approval {
    approved: bool,
}
#[derive(Deserialize)]
struct RunReq {
    #[serde(rename = "experimentId")]
    experiment_id: String,
    approved: bool,
    #[serde(rename = "expectedCommit")]
    expected_commit: Option<String>,
    #[serde(rename = "targetId")]
    target_id: Option<String>,
}
#[derive(Deserialize)]
struct LogQuery {
    start: Option<u64>,
    end: Option<u64>,
    ident: Option<String>,
}

pub fn routes(bin: Option<PathBuf>, state_dir: PathBuf) -> Router {
    routes_with_policy(bin, state_dir, crate::ports::port_policy([]))
}

pub fn routes_with_policy(
    bin: Option<PathBuf>,
    state_dir: PathBuf,
    blocked_ports: crate::ports::PortPolicy,
) -> Router {
    routes_for_service(ResearchService::new(bin, state_dir, blocked_ports))
}
pub(crate) fn routes_for_service(state: ResearchService) -> Router {
    Router::new()
        .route("/api/research/status", get(status))
        .route("/api/research/connect", post(connect))
        .route("/api/research/projects", get(projects).post(create_project))
        .route("/api/research/projects/{id}", get(project))
        .route(
            "/api/research/projects/{id}/experiments",
            get(experiments).post(create_experiment),
        )
        .route(
            "/api/research/projects/{id}/runs",
            get(runs).post(create_run),
        )
        .route(
            "/api/research/projects/{id}/runs/{run_id}/cancel",
            post(cancel_run),
        )
        .route(
            "/api/research/projects/{id}/runs/{run_id}/log",
            get(run_log),
        )
        .route("/api/research/projects/{id}/files", get(files))
        .with_state(state)
}
async fn status(State(s): State<ResearchState>) -> Json<Value> {
    let mut status = serde_json::to_value(inspect(&s).await).expect("status serialization");
    if status["connected"] == true
        && let Ok(caps) = s.capabilities().await
    {
        status["capabilities"] = caps;
    }
    Json(status)
}
async fn connect(State(s): State<ResearchState>) -> Response {
    let st = inspect(&s).await;
    if st.connected {
        return status(State(s)).await.into_response();
    }
    if !st.available {
        return err(
            StatusCode::SERVICE_UNAVAILABLE,
            st.error
                .unwrap_or_else(|| "OpenResearch unavailable".into()),
        );
    }
    if matches!(descriptor(&s).await, Ok(Some(d)) if !process_gone(d.pid)) {
        return err(
            StatusCode::CONFLICT,
            "An OpenResearch descriptor already exists but failed validation. Do not start another server; inspect the runtime first.",
        );
    }
    if let Err(e) = start(&s).await {
        return err(StatusCode::SERVICE_UNAVAILABLE, e);
    }
    match descriptor(&s).await {
        Ok(Some(d)) => match validate(&s, &d).await {
            Ok(_) => status(State(s)).await.into_response(),
            Err(e) => err(StatusCode::SERVICE_UNAVAILABLE, e),
        },
        _ => err(
            StatusCode::SERVICE_UNAVAILABLE,
            "OpenResearch did not publish a valid server descriptor",
        ),
    }
}
async fn inspect(s: &ResearchState) -> Status {
    let Some(bin) = s.bin.as_ref() else {
        return Status {
            available: false,
            connected: false,
            version: None,
            error: Some("OpenResearch is not configured. Set --openresearch-bin.".into()),
        };
    };
    let out = tokio::time::timeout(
        Duration::from_secs(3),
        isolated_command(bin).arg("--version").output(),
    )
    .await;
    let version = match out {
        Ok(Ok(o)) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        _ => {
            return Status {
                available: false,
                connected: false,
                version: None,
                error: Some("Configured OpenResearch binary could not be executed.".into()),
            };
        }
    };
    let available = version.split_whitespace().last() == Some(EXPECTED_VERSION);
    let (connected, error) = if !available {
        (
            false,
            Some("OpenResearch binary version must be 0.2.27.".into()),
        )
    } else {
        match descriptor(s).await {
            Ok(Some(d)) => match validate(s, &d).await {
                Ok(_) => (true, None),
                Err(error) => (false, Some(error)),
            },
            Ok(None) => (
                false,
                Some("OpenResearch is not connected. Select Connect to start it.".into()),
            ),
            Err(error) => (false, Some(error)),
        }
    };
    Status {
        available,
        connected,
        version: Some(version),
        error,
    }
}
async fn start(s: &ResearchState) -> Result<(), String> {
    tokio::fs::create_dir_all(&s.root)
        .await
        .map_err(|e| format!("Could not prepare isolated research settings: {e}"))?;
    let home = s.root.join("home");
    let config = s.root.join("config");
    let data = s.root.join("data");
    let cache = s.root.join("cache");
    for p in [&home, &config, &data, &cache] {
        tokio::fs::create_dir_all(p)
            .await
            .map_err(|e| format!("Could not prepare isolated research settings: {e}"))?;
    }
    let settings = config.join("openresearch/settings.json");
    tokio::fs::create_dir_all(settings.parent().unwrap())
        .await
        .map_err(|e| e.to_string())?;
    tokio::fs::write(settings, r#"{"telemetryDisabled":true,"autoUpdate":false}"#)
        .await
        .map_err(|e| e.to_string())?;
    let bin = s.bin.as_ref().ok_or("OpenResearch is not configured")?;
    let mut lock = s.child.lock().await;
    if let Some(child) = lock.as_mut() {
        match child.try_wait() {
            Ok(None) => return Err("OpenResearch start was already requested; inspect connection status before retrying.".into()),
            Ok(Some(_)) | Err(_) => { *lock = None; }
        }
    }
    if lock.as_ref().is_some_and(|c| c.id().is_some()) {
        return Err(
            "OpenResearch start was already requested; inspect connection status before retrying."
                .into(),
        );
    }
    let reservation = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .map_err(|e| format!("Could not reserve OpenResearch port: {e}"))?;
    let port = reservation.local_addr().map_err(|e| e.to_string())?.port();
    // ponytail: Retain port blocks until node restart; reclaim only with verified listener ownership.
    s.blocked_ports
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .insert(port);
    let metrics_reservation = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .map_err(|e| e.to_string())?;
    let metrics_port = metrics_reservation
        .local_addr()
        .map_err(|e| e.to_string())?
        .port();
    s.blocked_ports
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .insert(metrics_port);
    drop(metrics_reservation);
    drop(reservation);
    #[expect(
        clippy::disallowed_methods,
        reason = "the OpenResearch runtime is a node-owned service that outlives node restarts; it is tracked by its runtime descriptor and stopped by PID"
    )]
    let c = isolated_command(bin)
        .args([
            "--no-telemetry",
            "up",
            "--host",
            "127.0.0.1",
            "--port",
            &port.to_string(),
            "--no-browser",
            "--no-agent",
        ])
        .env("ORX_POSTUDIO_TRACKIO_PORT", metrics_port.to_string())
        .env("HOME", home)
        .env("ORX_DATA_DIR", data)
        .env("ORX_CACHE_DIR", cache)
        .env("XDG_CONFIG_HOME", config)
        .env("ORX_NO_UPDATE_CHECK", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(false)
        .spawn()
        .map_err(|e| format!("Could not start OpenResearch: {e}"))?;
    *lock = Some(c);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        if let Ok(Some(d)) = descriptor(s).await
            && validate(s, &d).await.is_ok()
        {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("OpenResearch start requested but no authenticated server became ready; inspect status before retrying.".into());
        }
        tokio::time::sleep(Duration::from_millis(150)).await
    }
}
async fn descriptor(s: &ResearchState) -> Result<Option<Descriptor>, String> {
    descriptor_at(&s.root).await
}

pub async fn existing_runtime_ports(state_dir: &Path) -> Result<Vec<u16>, String> {
    let Some(d) = descriptor_at(&state_dir.join("research/openresearch")).await? else {
        return Ok(vec![]);
    };
    let ports: Vec<_> = std::iter::once(d.port).chain(d.private_ports).collect();
    if ports.contains(&0) {
        return Err("Unsafe OpenResearch server descriptor.".into());
    }
    Ok(ports)
}

async fn descriptor_at(root: &Path) -> Result<Option<Descriptor>, String> {
    let data = root.join("data");
    match tokio::fs::metadata(&data).await {
        Ok(meta) if meta.is_dir() => {}
        Ok(_) => return Err("Research data path is not a directory.".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "Could not inspect research data directory: {error}"
            ));
        }
    }
    let canonical = canonicalize(data.clone())
        .await
        .map_err(|e| format!("Research data directory unavailable: {e}"))?;
    let hash = digest::digest(&digest::SHA256, canonical.to_string_lossy().as_bytes());
    let name = hash.as_ref()[..16]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let path = if cfg!(unix) {
        PathBuf::from(format!("/tmp/orx-{}", libc_uid())).join(format!("{name}.server.json"))
    } else {
        canonical.join("runtime/server.json")
    };
    #[cfg(unix)]
    if let Some(parent) = path.parent() {
        use std::os::unix::fs::MetadataExt;
        let parent_meta = match tokio::fs::symlink_metadata(parent).await {
            Ok(meta) => meta,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(format!(
                    "Could not inspect OpenResearch runtime directory: {error}"
                ));
            }
        };
        if parent_meta.file_type().is_symlink()
            || !parent_meta.is_dir()
            || parent_meta.uid() != libc_uid()
            || parent_meta.mode() & 0o077 != 0
        {
            return Err("Unsafe OpenResearch runtime directory permissions.".into());
        }
    }
    let meta = match tokio::fs::symlink_metadata(&path).await {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("Could not inspect OpenResearch descriptor: {e}")),
    };
    if meta.file_type().is_symlink() || !meta.is_file() || meta.len() > 4096 {
        return Err("Unsafe OpenResearch server descriptor.".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.uid() != libc_uid() || meta.mode() & 0o077 != 0 {
            return Err("Unsafe OpenResearch server descriptor permissions.".into());
        }
    }
    let bytes = tokio::fs::read(path)
        .await
        .map_err(|e| format!("Could not read OpenResearch descriptor: {e}"))?;
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| "Invalid OpenResearch server descriptor.".into())
}
fn process_gone(pid: u32) -> bool {
    #[cfg(unix)]
    if let Ok(pid) = i32::try_from(pid) {
        return pid > 0
            && unsafe { libc::kill(pid, 0) } == -1
            && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH);
    }
    false
}
#[cfg(unix)]
fn libc_uid() -> u32 {
    unsafe { libc::geteuid() }
}
#[cfg(not(unix))]
fn libc_uid() -> u32 {
    0
}
async fn validate(s: &ResearchState, d: &Descriptor) -> Result<Value, String> {
    if d.proto != 2 || d.version != EXPECTED_VERSION || d.port == 0 || d.token.is_empty() {
        return Err("OpenResearch descriptor version or fields unsupported.".into());
    }
    let nonce = random_nonce()?;
    let url = format!("http://127.0.0.1:{}/api/v2/hello?nonce={nonce}", d.port);
    let r = s
        .client
        .get(url)
        .send()
        .await
        .map_err(|_| "OpenResearch server is not reachable.".to_string())?;
    if !r.status().is_success() {
        return Err("OpenResearch hello validation failed.".into());
    }
    let mut r = r;
    let mut bytes = Vec::new();
    while let Some(chunk) = r
        .chunk()
        .await
        .map_err(|_| "OpenResearch hello response invalid.".to_string())?
    {
        if bytes.len().saturating_add(chunk.len()) > BODY_LIMIT {
            return Err("OpenResearch hello response exceeded size limit.".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    let v: Value = serde_json::from_slice(&bytes)
        .map_err(|_| "OpenResearch hello response invalid.".to_string())?;
    let proof = v["proof"]
        .as_str()
        .ok_or("OpenResearch hello proof missing")?;
    let signature = STANDARD
        .decode(proof)
        .map_err(|_| "OpenResearch hello proof was malformed")?;
    if v["instance"].as_str() != Some(&d.instance)
        || v["proto"].as_u64() != Some(2)
        || hmac::verify(
            &hmac::Key::new(hmac::HMAC_SHA256, d.token.as_bytes()),
            &{
                let mut message = d.instance.as_bytes().to_vec();
                message.push(0);
                message.extend_from_slice(nonce.as_bytes());
                message
            },
            &signature,
        )
        .is_err()
    {
        return Err("OpenResearch hello authentication failed.".into());
    }
    {
        let mut blocked = s
            .blocked_ports
            .write()
            .unwrap_or_else(|error| error.into_inner());
        blocked.insert(d.port);
        blocked.extend(d.private_ports.iter().copied());
    }
    Ok(v)
}
fn random_nonce() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    ring::rand::SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| "Could not generate OpenResearch challenge.".to_string())?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes))
}
fn isolated_command(bin: &Path) -> Command {
    let mut command = Command::new(bin);
    command.env_clear().kill_on_drop(true);
    for key in ["PATH", "TMPDIR"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    #[cfg(windows)]
    for key in ["SystemRoot", "WINDIR", "COMSPEC", "PATHEXT"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command
}
async fn upstream(
    s: &ResearchState,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> Result<Value, Response> {
    let d = descriptor(s)
        .await
        .map_err(|e| err(StatusCode::SERVICE_UNAVAILABLE, e))?
        .ok_or_else(|| {
            err(
                StatusCode::SERVICE_UNAVAILABLE,
                "OpenResearch is not connected.",
            )
        })?;
    validate(s, &d)
        .await
        .map_err(|e| err(StatusCode::SERVICE_UNAVAILABLE, e))?;
    let url = format!(
        "http://127.0.0.1:{}/{}",
        d.port,
        path.trim_start_matches('/')
    );
    let mut req = s.client.request(method, url).bearer_auth(&d.token);
    if let Some(b) = body {
        req = req.json(&b)
    }
    let response = req.send().await.map_err(|_| {
        err(
            StatusCode::SERVICE_UNAVAILABLE,
            "OpenResearch request failed; mutation outcome may be unknown.",
        )
    })?;
    let status = response.status();
    if response
        .content_length()
        .is_some_and(|len| len > BODY_LIMIT as u64)
    {
        return Err(err(
            StatusCode::BAD_GATEWAY,
            "OpenResearch response exceeded size limit",
        ));
    }
    let mut bytes = Vec::new();
    let mut response = response;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| err(StatusCode::BAD_GATEWAY, "OpenResearch response unreadable"))?
    {
        if bytes.len().saturating_add(chunk.len()) > BODY_LIMIT {
            return Err(err(
                StatusCode::BAD_GATEWAY,
                "OpenResearch response exceeded size limit",
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| {
        err(
            StatusCode::BAD_GATEWAY,
            "OpenResearch returned invalid JSON",
        )
    })?;
    if !status.is_success() {
        return Err(err(
            StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY),
            value["error"]
                .as_str()
                .unwrap_or("OpenResearch request failed")
                .to_owned(),
        ));
    }
    Ok(value)
}
async fn projects(State(s): State<ResearchState>) -> Response {
    api(upstream(&s, Method::GET, "api/projects", None)
        .await
        .map(|v| json!({"projects":v["projects"]})))
}
async fn create_project(State(s): State<ResearchState>, Json(r): Json<ProjectReq>) -> Response {
    if r.name.trim().is_empty() {
        return err(StatusCode::BAD_REQUEST, "Project name required");
    }
    let p = Path::new(&r.path);
    if !p.is_absolute() || !p.is_dir() || !git_repo(p).await {
        return err(
            StatusCode::BAD_REQUEST,
            "Path must be an existing Git repository",
        );
    }
    // An explicit unsupported starter agent makes the upstream warm hook a no-op; absent selection auto-detects an agent.
    let body = json!({"name":r.name,"path":r.path,"createFolder":false,"initializeGit":false,"githubSyncEnabled":false,"locale":"ko","starterAgent":{"harness":"","profile":null,"model":null}});
    api(upstream(&s, Method::POST, "api/projects", Some(body)).await)
}
/// `dunce::canonicalize` on the blocking pool: no `\\?\` verbatim paths on Windows.
async fn canonicalize(path: PathBuf) -> std::io::Result<PathBuf> {
    tokio::task::spawn_blocking(move || dunce::canonicalize(path))
        .await
        .map_err(std::io::Error::other)?
}
async fn git_repo(p: &Path) -> bool {
    let Ok(path) = canonicalize(p.to_path_buf()).await else {
        return false;
    };
    let output = tokio::time::timeout(
        Duration::from_secs(3),
        Command::new("git")
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .arg("-C")
            .arg(&path)
            .args(["rev-parse", "--show-toplevel"])
            .stderr(Stdio::null())
            .output(),
    )
    .await;
    match output {
        Ok(Ok(out)) if out.status.success() => {
            canonicalize(PathBuf::from(String::from_utf8_lossy(&out.stdout).trim()))
                .await
                .is_ok_and(|root| root == path)
        }
        _ => false,
    }
}
async fn project(State(s): State<ResearchState>, AxumPath(id): AxumPath<String>) -> Response {
    api(upstream(&s, Method::GET, &format!("api/projects/{}", enc(&id)), None).await)
}
async fn experiments(State(s): State<ResearchState>, AxumPath(id): AxumPath<String>) -> Response {
    api(upstream(
        &s,
        Method::GET,
        &format!("api/projects/{}/experiments", enc(&id)),
        None,
    )
    .await)
}
async fn create_experiment(
    State(s): State<ResearchState>,
    AxumPath(id): AxumPath<String>,
    Json(r): Json<ExperimentReq>,
) -> Response {
    if r.title.trim().is_empty() {
        return err(StatusCode::BAD_REQUEST, "Experiment title required");
    }
    let p = match upstream(&s, Method::GET, &format!("api/projects/{}", enc(&id)), None).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    let _ = p;
    if let Some(parent) = r.parent_id.as_ref() {
        let xs = match upstream(
            &s,
            Method::GET,
            &format!("api/projects/{}/experiments", enc(&id)),
            None,
        )
        .await
        {
            Ok(v) => v,
            Err(e) => return e,
        };
        if !xs["experiments"]
            .as_array()
            .is_some_and(|a| a.iter().any(|x| x["id"].as_str() == Some(parent)))
        {
            return err(
                StatusCode::BAD_REQUEST,
                "Parent experiment must belong to this project",
            );
        }
    }
    let Some(bin) = s.bin.as_ref() else {
        return err(
            StatusCode::SERVICE_UNAVAILABLE,
            "OpenResearch CLI unavailable",
        );
    };
    let mut cmd = isolated_command(bin);
    cmd.arg("--no-telemetry")
        .arg("create-experiment")
        .arg(&id)
        .arg("--title")
        .arg(r.title);
    if let Some(v) = r.description {
        cmd.arg("--description").arg(v);
    }
    if let Some(v) = r.run_command {
        cmd.arg("--run-command").arg(v);
    }
    if let Some(v) = r.parent_id {
        cmd.arg("--parent").arg(v);
    } else {
        cmd.arg("--baseline");
    }
    #[expect(
        clippy::disallowed_methods,
        reason = "bounded orx CLI call with kill_on_drop; the child cannot outlive this request"
    )]
    let mut child = match cmd
        .env("HOME", s.root.join("home"))
        .env("ORX_DATA_DIR", s.root.join("data"))
        .env("ORX_CACHE_DIR", s.root.join("cache"))
        .env("XDG_CONFIG_HOME", s.root.join("config"))
        .env("ORX_NO_UPDATE_CHECK", "1")
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            return err(
                StatusCode::SERVICE_UNAVAILABLE,
                format!("Could not create experiment: {e}"),
            );
        }
    };
    match tokio::time::timeout(Duration::from_secs(30), child.wait()).await {
        Ok(Ok(status)) if status.success() => {}
        Ok(Ok(_)) => {
            return err(
                StatusCode::BAD_REQUEST,
                "OpenResearch could not create experiment",
            );
        }
        _ => {
            return err(
                StatusCode::GATEWAY_TIMEOUT,
                "Experiment creation timed out; refresh before retrying",
            );
        }
    };
    experiments(State(s), AxumPath(id)).await
}
async fn runs(State(s): State<ResearchState>, AxumPath(id): AxumPath<String>) -> Response {
    api(upstream(
        &s,
        Method::GET,
        &format!("api/v2/runs?project={}", urlencoding::encode(&id)),
        None,
    )
    .await
    .map(|v| json!({"runs":v["runs"]})))
}
async fn create_run(
    State(s): State<ResearchState>,
    AxumPath(id): AxumPath<String>,
    Json(r): Json<RunReq>,
) -> Response {
    if !r.approved {
        return err(StatusCode::BAD_REQUEST, "Explicit run approval required");
    }
    let Some(expected) = r.expected_commit.as_deref() else {
        return err(StatusCode::BAD_REQUEST, "expectedCommit is required");
    };
    let _guard = s.run_submission.lock().await;
    api(s
        .submit_run(
            &id,
            &r.experiment_id,
            expected,
            r.target_id.as_deref().unwrap_or("local"),
            None,
        )
        .await
        .map(|run| json!({"run":run})))
}
async fn cancel_run(
    State(s): State<ResearchState>,
    AxumPath((id, run_id)): AxumPath<(String, String)>,
    Json(a): Json<Approval>,
) -> Response {
    if !a.approved {
        return err(
            StatusCode::BAD_REQUEST,
            "Explicit cancellation approval required",
        );
    }
    let v = match upstream(
        &s,
        Method::GET,
        &format!("api/v2/runs/{}", enc(&run_id)),
        None,
    )
    .await
    {
        Ok(v) => v,
        Err(e) => return e,
    };
    if v["projectId"] != id {
        return err(StatusCode::NOT_FOUND, "Run not found in this project");
    }
    api(upstream(
        &s,
        Method::POST,
        &format!("api/v2/runs/{}/cancel", enc(&run_id)),
        Some(json!({})),
    )
    .await
    .map(|run| json!({"run": run})))
}
async fn run_log(
    State(s): State<ResearchState>,
    AxumPath((id, run_id)): AxumPath<(String, String)>,
    Query(q): Query<LogQuery>,
) -> Response {
    let v = match upstream(
        &s,
        Method::GET,
        &format!("api/v2/runs/{}", enc(&run_id)),
        None,
    )
    .await
    {
        Ok(v) => v,
        Err(e) => return e,
    };
    if v["projectId"] != id {
        return err(StatusCode::NOT_FOUND, "Run not found in this project");
    }
    let start = q.start.unwrap_or(0);
    let end = q
        .end
        .unwrap_or(start.saturating_add(65536))
        .min(start.saturating_add(65536));
    if end < start {
        return err(StatusCode::BAD_REQUEST, "Invalid log range");
    }
    let mut path = format!("api/v2/runs/{}/log?start={start}&end={end}", enc(&run_id));
    if let Some(i) = q.ident {
        path.push_str("&ident=");
        path.push_str(&urlencoding::encode(&i))
    }
    let data = match upstream(&s, Method::GET, &path, None).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    let (start, ident, bytes, total) = match data["kind"].as_str() {
        Some("data") => (
            data["start"].as_u64().unwrap_or(start),
            data["ident"].clone(),
            data["data"].as_str().map(str::to_owned),
            data["len"].as_u64().map(|_| data["len"].clone()),
        ),
        Some("reset") => match data["info"].as_object() {
            Some(info) => (
                start,
                info.get("ident").cloned().unwrap_or(Value::Null),
                Some(String::new()),
                info.get("len").cloned(),
            ),
            None => {
                return err(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "Run log is not available yet.",
                );
            }
        },
        _ => {
            return err(
                StatusCode::BAD_GATEWAY,
                "OpenResearch returned an unknown log page.",
            );
        }
    };
    let Some(bytes) = bytes else {
        return err(
            StatusCode::BAD_GATEWAY,
            "OpenResearch log page lacked data.",
        );
    };
    let bytes = match STANDARD.decode(bytes) {
        Ok(bytes) => bytes,
        Err(_) => {
            return err(
                StatusCode::BAD_GATEWAY,
                "OpenResearch log page had invalid data.",
            );
        }
    };
    api(Ok(
        json!({"text":String::from_utf8_lossy(&bytes),"start":start,"end":start.saturating_add(bytes.len() as u64),"ident":ident,"total":total,"reset":data["kind"] == "reset"}),
    ))
}
async fn files(State(s): State<ResearchState>, AxumPath(id): AxumPath<String>) -> Response {
    api(upstream(
        &s,
        Method::GET,
        &format!("api/projects/{}/files", enc(&id)),
        None,
    )
    .await
    .map(|v| {
        let mut files = Vec::new();
        let mut pending = v["entries"].as_array().cloned().unwrap_or_default();
        while let Some(entry) = pending.pop() {
            if let Some(children) = entry["children"].as_array() {
                pending.extend(children.iter().cloned());
            }
            files.push(json!({
                "name": entry["name"], "path": entry["path"], "size": entry["size"],
                "kind": if entry["isDir"] == true { "directory" } else { "file" }
            }));
        }
        files.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
        json!({"files":files})
    }))
}
fn enc(s: &str) -> String {
    urlencoding::encode(s).into_owned()
}
fn api(r: Result<Value, Response>) -> Response {
    match r {
        Ok(v) => Json(v).into_response(),
        Err(e) => e,
    }
}
fn err(status: StatusCode, msg: impl Into<String>) -> Response {
    (status, Json(json!({"error":msg.into()}))).into_response()
}

#[expect(
    clippy::disallowed_methods,
    reason = "loopback-only client for the node-owned OpenResearch runtime on 127.0.0.1; proxies and redirects are disabled"
)]
fn loopback_client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()
        .expect("static client config")
}

impl ResearchState {
    pub(crate) fn new(
        bin: Option<PathBuf>,
        state_dir: PathBuf,
        blocked_ports: crate::ports::PortPolicy,
    ) -> Self {
        Self {
            bin,
            root: state_dir.join("research/openresearch"),
            child: Arc::new(Mutex::new(None)),
            run_submission: Arc::new(Mutex::new(())),
            blocked_ports,
            refs: crate::research_refs::RefsStore::new(state_dir.clone()),
            client: loopback_client(),
            papers: crate::research_library::PaperCache::default(),
        }
    }
    pub(crate) fn papers(&self) -> &crate::research_library::PaperCache {
        &self.papers
    }
    /// One read-only `orx` call in the isolated research home. Stdout is capped at `limit`
    /// bytes; a timeout drops the child, which `kill_on_drop` reaps.
    pub(crate) async fn orx_output(
        &self,
        args: &[&str],
        limit: usize,
        timeout: Duration,
    ) -> Result<Vec<u8>, OrxCallError> {
        use tokio::io::AsyncReadExt;
        let bin = self.bin.as_ref().ok_or(OrxCallError::NotConfigured)?;
        let home = self.root.join("home");
        tokio::fs::create_dir_all(&home)
            .await
            .map_err(|_| OrxCallError::Launch)?;
        #[expect(
            clippy::disallowed_methods,
            reason = "bounded read-only orx call with kill_on_drop; the timeout reaps it"
        )]
        let mut child = isolated_command(bin)
            .args(args)
            .env("HOME", home)
            .env("XDG_CONFIG_HOME", self.root.join("config"))
            .env("ORX_DATA_DIR", self.root.join("data"))
            .env("ORX_CACHE_DIR", self.root.join("cache"))
            .env("ORX_NO_UPDATE_CHECK", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|_| OrxCallError::Launch)?;
        let stdout = child.stdout.take().ok_or(OrxCallError::Launch)?;
        tokio::time::timeout(timeout, async {
            let mut bytes = Vec::new();
            stdout
                .take(limit as u64 + 1)
                .read_to_end(&mut bytes)
                .await
                .map_err(|_| OrxCallError::Read)?;
            if bytes.len() > limit {
                return Err(OrxCallError::TooLarge);
            }
            if !child.wait().await.is_ok_and(|s| s.success()) {
                return Err(OrxCallError::Failed);
            }
            Ok(bytes)
        })
        .await
        .map_err(|_| OrxCallError::Timeout)?
    }
    pub(crate) fn state_dir(&self) -> PathBuf {
        self.root
            .parent()
            .and_then(Path::parent)
            .expect("research root")
            .to_path_buf()
    }
    pub(crate) async fn native_json(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value, Response> {
        upstream(self, method, path, body).await
    }
    pub(crate) async fn owned_project(&self, p: &str) -> Result<Value, Response> {
        self.native_json(Method::GET, &format!("api/projects/{}", enc(p)), None)
            .await
    }
    pub(crate) async fn owned_experiment(&self, p: &str, e: &str) -> Result<Value, Response> {
        self.owned_project(p).await?;
        let xs = self
            .native_json(
                Method::GET,
                &format!("api/projects/{}/experiments", enc(p)),
                None,
            )
            .await?;
        xs["experiments"]
            .as_array()
            .and_then(|xs| xs.iter().find(|x| x["id"].as_str() == Some(e)))
            .cloned()
            .ok_or_else(|| {
                err(
                    StatusCode::NOT_FOUND,
                    "Experiment not found in this project",
                )
            })
    }
    pub(crate) async fn owned_run(&self, p: &str, r: &str) -> Result<Value, Response> {
        let v = self
            .native_json(Method::GET, &format!("api/v2/runs/{}", enc(r)), None)
            .await?;
        let run = v.get("run").unwrap_or(&v);
        if run["projectId"].as_str() != Some(p) {
            return Err(err(StatusCode::NOT_FOUND, "Run not found in this project"));
        }
        Ok(run.clone())
    }
    pub(crate) async fn refs_get(
        &self,
        p: &str,
    ) -> Result<crate::research_refs::ProjectResearchRefs, Response> {
        self.owned_project(p).await?;
        self.refs
            .get(p)
            .await
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))
    }
    pub(crate) async fn refs_update(
        &self,
        p: &str,
        update: crate::research_refs::RefsUpdate,
    ) -> Result<crate::research_refs::ProjectResearchRefs, Response> {
        self.owned_project(p).await?;
        self.refs
            .update(p, update)
            .await
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))
    }
    pub(crate) async fn refs_modify(
        &self,
        p: &str,
        update: impl FnOnce(&mut crate::research_refs::ProjectResearchRefs) -> Result<(), String>,
    ) -> Result<crate::research_refs::ProjectResearchRefs, Response> {
        self.owned_project(p).await?;
        let mut failure = None;
        let value = self
            .refs
            .modify(p, |refs| {
                let mut next = refs.clone();
                match update(&mut next) {
                    Ok(()) => *refs = next,
                    Err(error) => failure = Some(error),
                }
            })
            .await
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))?;
        if let Some(error) = failure {
            return Err(err(StatusCode::BAD_REQUEST, error));
        }
        Ok(value)
    }
    pub(crate) async fn discover_papers(
        &self,
        source: &str,
        query: &str,
    ) -> Result<Value, Response> {
        let strategy = match source {
            "alphaXiv" => "keyword",
            "openAlex" => "openalex",
            "bioRxiv" => "biorxiv",
            "pubMed" => "pubmed",
            _ => return Err(err(StatusCode::BAD_REQUEST, "Invalid literature source")),
        };
        if query.trim().is_empty() || query.len() > 512 {
            return Err(err(StatusCode::BAD_REQUEST, "Invalid search terms"));
        }
        let bytes = self
            .orx_output(
                &[
                    "--no-telemetry",
                    "discover",
                    strategy,
                    "--limit",
                    "15",
                    "--",
                    query,
                ],
                BODY_LIMIT,
                Duration::from_secs(16),
            )
            .await
            .map_err(|error| match error {
                OrxCallError::NotConfigured => err(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "OpenResearch not configured",
                ),
                OrxCallError::Launch => err(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "Could not launch literature search",
                ),
                OrxCallError::Read => {
                    err(StatusCode::BAD_GATEWAY, "Could not read literature results")
                }
                OrxCallError::TooLarge => err(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "Literature response too large",
                ),
                OrxCallError::Invalid | OrxCallError::Failed => err(
                    StatusCode::BAD_GATEWAY,
                    "Literature provider unavailable or disabled",
                ),
                OrxCallError::Timeout => {
                    err(StatusCode::GATEWAY_TIMEOUT, "Literature provider timed out")
                }
            })?;
        serde_json::from_slice(&bytes)
            .map_err(|_| err(StatusCode::BAD_GATEWAY, "Invalid literature results"))
    }
    pub(crate) async fn metrics(&self, p: &str, r: &str) -> Result<Value, Response> {
        self.owned_run(p, r).await?;
        let native = self
            .native_json(
                Method::GET,
                &format!("api/trackio/projects/{}/runs/{}/metrics", enc(p), enc(r)),
                None,
            )
            .await?;
        let mut metrics = serde_json::Map::new();
        let mut series = serde_json::Map::new();
        for metric in native["metrics"].as_array().into_iter().flatten() {
            let Some(name) = metric["name"].as_str() else {
                continue;
            };
            if let Some(value) = metric["last"]["value"].as_f64().filter(|v| v.is_finite()) {
                metrics.insert(name.into(), json!(value));
            }
            let points: Vec<_> = metric["series"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|p| {
                    p["value"].as_f64().is_some_and(f64::is_finite)
                        && p["step"].as_f64().is_some_and(f64::is_finite)
                })
                .cloned()
                .collect();
            series.insert(name.into(), json!(points));
        }
        Ok(
            json!({"runId":r,"metrics":metrics,"series":series,"source":"trackio","sourceUrl":null,"errors":native["errors"].as_array().cloned().unwrap_or_default()}),
        )
    }
    pub(crate) async fn capabilities(&self) -> Result<Value, Response> {
        let d = descriptor(self)
            .await
            .map_err(|e| err(StatusCode::SERVICE_UNAVAILABLE, e))?
            .ok_or_else(|| err(StatusCode::SERVICE_UNAVAILABLE, "Engine unavailable"))?;
        let hello = validate(self, &d)
            .await
            .map_err(|e| err(StatusCode::SERVICE_UNAVAILABLE, e))?;
        let contract = hello["postudioContract"].as_u64().unwrap_or(0);
        Ok(
            json!({"postudioContract":contract,"expectedCommit":contract>=1,"workspace":contract>=1,"metrics":contract>=1,"sources":["alphaXiv","openAlex","bioRxiv","pubMed"],"dashboard":std::env::var("POSTUDIO_RESEARCH_DASHBOARD_ORIGIN").is_ok(),"researchTools":contract>=1,"approvedSshTarget":hello["capabilities"].as_array().is_some_and(|features|features.iter().any(|feature|feature=="approvedSshTarget"))}),
        )
    }
    pub(crate) async fn workspace(&self, p: &str, e: &str) -> Result<Value, Response> {
        self.owned_experiment(p, e).await?;
        self.native_json(
            Method::POST,
            &format!(
                "api/v2/projects/{}/experiments/{}/workspace",
                enc(p),
                enc(e)
            ),
            Some(json!({})),
        )
        .await
    }
    pub(crate) async fn create_experiment(&self, p: &str, input: Value) -> Result<Value, Response> {
        let req: ExperimentReq = serde_json::from_value(input)
            .map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))?;
        let response =
            create_experiment(State(self.clone()), AxumPath(p.to_owned()), Json(req)).await;
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), BODY_LIMIT)
            .await
            .map_err(|_| err(StatusCode::BAD_GATEWAY, "Invalid experiment response"))?;
        if !status.is_success() {
            return Err((status, bytes).into_response());
        }
        serde_json::from_slice(&bytes)
            .map_err(|_| err(StatusCode::BAD_GATEWAY, "Invalid experiment response"))
    }
    async fn submit_run(
        &self,
        p: &str,
        e: &str,
        expected: &str,
        target: &str,
        approved_target: Option<&Value>,
    ) -> Result<Value, Response> {
        self.owned_experiment(p, e).await?;
        let resolved = crate::research_targets::resolve(self, p, target).await?;
        if resolved["requiresApproval"] == true && approved_target.is_none() {
            return Err(err(
                StatusCode::CONFLICT,
                "Remote execution requires a stored owner approval",
            ));
        }
        if approved_target.is_some_and(|expected| expected != &resolved) {
            return Err(err(
                StatusCode::CONFLICT,
                "Execution target changed; request fresh approval",
            ));
        }
        if !matches!(expected.len(), 40 | 64) || !expected.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(err(StatusCode::BAD_REQUEST, "Invalid expected commit"));
        }
        let capabilities = self.capabilities().await?;
        if resolved["requiresApproval"] == true && capabilities["approvedSshTarget"] != true {
            return Err(err(
                StatusCode::CONFLICT,
                "OpenResearch approved SSH target contract upgrade required",
            ));
        }
        if capabilities["expectedCommit"] != true {
            return Err(err(
                StatusCode::CONFLICT,
                "OpenResearch snapshot contract upgrade required",
            ));
        }
        let mut body = json!({"exp_id":e,"backend":resolved["backend"],"no_container":false,"force":false,"forwarded":true,"telemetry_suppressed":true,"expected_commit":expected});
        if let Some(host) = resolved["host"].as_str() {
            body["host"] = json!(host);
            body["approved_ssh_target"] = approved_target
                .and_then(|target| target.get("hostSpec"))
                .cloned()
                .ok_or_else(|| err(StatusCode::CONFLICT, "Approved SSH target metadata missing"))?;
        }
        self.native_json(Method::POST, "api/v2/runs", Some(body))
            .await
    }
    pub(crate) async fn save_target_bindings(
        &self,
        p: &str,
        bindings: Vec<crate::research_refs::Binding>,
    ) -> Result<crate::research_refs::ProjectResearchRefs, Response> {
        let _gate = self.run_submission.lock().await;
        self.refs_modify(p, |refs| {
            refs.target_bindings = bindings;
            Ok(())
        })
        .await
    }
    pub(crate) async fn request_run_approval(
        &self,
        p: &str,
        e: &str,
        session: Option<&str>,
        target: &str,
        expected: &str,
    ) -> Result<Value, Response> {
        self.owned_experiment(p, e).await?;
        let target_spec = crate::research_targets::resolve(self, p, target).await?;
        if !matches!(expected.len(), 40 | 64) || !expected.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(err(
                StatusCode::BAD_REQUEST,
                "Invalid approved target or commit",
            ));
        }
        let approval = crate::research_refs::ResearchApproval {
            id: random_nonce().map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))?,
            project_id: p.into(),
            experiment_id: e.into(),
            session_id: session.map(str::to_owned),
            expected_commit: expected.into(),
            target_id: target.into(),
            target_spec: Some(target_spec),
            status: "pending".into(),
            run_id: None,
            error: None,
        };
        self.refs_modify(p, |refs| {
            if refs.approvals.len() >= 1000 {
                return Err(
                    "Too many approval requests; archive completed records before retrying".into(),
                );
            }
            refs.approvals.push(approval.clone());
            Ok(())
        })
        .await?;
        Ok(json!({"approval":approval}))
    }
    pub(crate) async fn execute_owner_approval(
        &self,
        p: &str,
        id: &str,
        approved: bool,
    ) -> Result<Value, Response> {
        let _guard = self.run_submission.lock().await;
        let mut approval = self
            .refs_get(p)
            .await?
            .approvals
            .into_iter()
            .find(|a| a.id == id)
            .ok_or_else(|| err(StatusCode::NOT_FOUND, "Approval not found"))?;
        if approval.status != "pending" {
            return Err(err(
                StatusCode::CONFLICT,
                "Approval has already been handled; refresh status",
            ));
        }
        if approved {
            let current = crate::research_targets::resolve(self, p, &approval.target_id).await?;
            if approval
                .target_spec
                .as_ref()
                .is_some_and(|expected| expected != &current)
            {
                return Err(err(
                    StatusCode::CONFLICT,
                    "Execution target changed; request fresh approval",
                ));
            }
        }
        approval.status = if approved { "approved" } else { "rejected" }.into();
        self.refs_modify(p, |refs| {
            *refs
                .approvals
                .iter_mut()
                .find(|a| a.id == id)
                .ok_or("Approval missing")? = approval.clone();
            Ok(())
        })
        .await?;
        let mut run = None;
        if approved {
            match self
                .submit_run(
                    p,
                    &approval.experiment_id,
                    &approval.expected_commit,
                    &approval.target_id,
                    approval.target_spec.as_ref(),
                )
                .await
            {
                Ok(value) => {
                    approval.status = "submitted".into();
                    approval.run_id = value.get("run").unwrap_or(&value)["id"]
                        .as_str()
                        .map(str::to_owned);
                    run = Some(value.get("run").unwrap_or(&value).clone());
                }
                Err(response) => {
                    approval.status = "failed".into();
                    approval.error = Some(format!(
                        "Execution submission returned {}; verify engine before making a new request",
                        response.status()
                    ));
                    self.refs_modify(p, |refs| {
                        *refs
                            .approvals
                            .iter_mut()
                            .find(|a| a.id == id)
                            .ok_or("Approval missing")? = approval.clone();
                        Ok(())
                    })
                    .await?;
                    return Err(response);
                }
            }
            self.refs_modify(p, |refs| {
                *refs
                    .approvals
                    .iter_mut()
                    .find(|a| a.id == id)
                    .ok_or("Approval missing")? = approval.clone();
                Ok(())
            })
            .await?;
        }
        Ok(json!({"approval":approval,"run":run}))
    }
    pub(crate) async fn native_bytes(
        &self,
        path: &str,
        limit: usize,
    ) -> Result<(String, Vec<u8>), Response> {
        let d = descriptor(self)
            .await
            .map_err(|e| err(StatusCode::SERVICE_UNAVAILABLE, e))?
            .ok_or_else(|| err(StatusCode::SERVICE_UNAVAILABLE, "Engine unavailable"))?;
        validate(self, &d)
            .await
            .map_err(|e| err(StatusCode::SERVICE_UNAVAILABLE, e))?;
        let url = format!(
            "http://127.0.0.1:{}/{}",
            d.port,
            path.trim_start_matches('/')
        );
        let mut response = self
            .client
            .get(url)
            .bearer_auth(d.token)
            .send()
            .await
            .map_err(|_| err(StatusCode::BAD_GATEWAY, "Engine request failed"))?;
        if !response.status().is_success() {
            return Err(err(response.status(), "Engine file unavailable"));
        }
        let mime = response
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_owned();
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| err(StatusCode::BAD_GATEWAY, "Engine response unreadable"))?
        {
            if bytes.len().saturating_add(chunk.len()) > limit {
                return Err(err(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "Engine response exceeds limit",
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok((mime, bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn missing_binary_is_unavailable() {
        let s = ResearchState {
            bin: None,
            root: PathBuf::from("/tmp/unneeded"),
            child: Arc::new(Mutex::new(None)),
            run_submission: Arc::new(Mutex::new(())),
            client: loopback_client(),
            blocked_ports: crate::ports::port_policy([]),
            refs: crate::research_refs::RefsStore::new(PathBuf::from("/tmp/unneeded")),
            papers: crate::research_library::PaperCache::default(),
        };
        let st = inspect(&s).await;
        assert!(!st.available);
        assert!(!st.connected);
        assert!(st.error.unwrap().contains("not configured"));
    }
    #[test]
    fn hello_uses_exact_hmac_message() {
        let nonce = "test-nonce";
        let d = Descriptor {
            pid: std::process::id(),
            instance: "inst".into(),
            port: 1,
            version: EXPECTED_VERSION.into(),
            proto: 2,
            token: "secret".into(),
            private_ports: vec![],
        };
        let mut m = hmac::Context::with_key(&hmac::Key::new(hmac::HMAC_SHA256, d.token.as_bytes()));
        m.update(d.instance.as_bytes());
        m.update(&[0]);
        m.update(nonce.as_bytes());
        assert_eq!(STANDARD.encode(m.sign().as_ref()).len(), 44);
    }
}
