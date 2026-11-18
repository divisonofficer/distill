use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::header::{AUTHORIZATION, CONTENT_LENGTH, ORIGIN};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use futures_util::StreamExt as _;
use ring::digest::{SHA256, digest};
use ring::hmac::{HMAC_SHA256, Key as HmacKey, sign as hmac_sign, verify as hmac_verify};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::chat_store::{Envelope, Store};

pub const MAX_SYNC_BODY_BYTES: usize = 16 * 1024 * 1024;
const MAX_CONFIG_BYTES: usize = 1024 * 1024;
const PEER_TIMEOUT: Duration = Duration::from_secs(10);
const SYNC_INTERVAL: Duration = Duration::from_secs(15);

#[derive(Clone)]
pub struct SyncStatus {
    inner: Arc<Mutex<SyncHealth>>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncHealth {
    pub available: bool,
    pub peer_count: usize,
    pub last_attempt_unix_seconds: Option<u64>,
    pub last_success_unix_seconds: Option<u64>,
    pub last_error: Option<&'static str>,
}

impl SyncStatus {
    pub fn disabled() -> Self {
        Self {
            inner: Arc::new(Mutex::new(SyncHealth {
                available: false,
                peer_count: 0,
                last_attempt_unix_seconds: None,
                last_success_unix_seconds: None,
                last_error: None,
            })),
        }
    }

    fn configured(peer_count: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(SyncHealth {
                available: peer_count != 0,
                peer_count,
                last_attempt_unix_seconds: None,
                last_success_unix_seconds: None,
                last_error: None,
            })),
        }
    }

    pub fn snapshot(&self) -> SyncHealth {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn record_round(&self, success: bool) {
        let mut status = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        status.last_attempt_unix_seconds = Some(now);
        if success {
            status.last_success_unix_seconds = Some(now);
            status.last_error = None;
        } else {
            status.last_error = Some("peer_exchange_failed");
        }
    }
}

#[derive(Clone)]
pub struct PeerSync {
    inner: Arc<PeerSyncInner>,
}

struct PeerSyncInner {
    store: Store,
    snapshot: Mutex<Arc<PeerSnapshot>>,
    changes: tokio::sync::watch::Sender<()>,
    client: reqwest::Client,
    status: SyncStatus,
}

struct PeerSnapshot {
    configs: Vec<PeerConfig>,
    peers: Vec<Peer>,
    trusted_keys: BTreeMap<String, String>,
}

struct Peer {
    incoming_token_tag: [u8; 32],
    outgoing_url: Url,
    outgoing_token: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ConfigFile {
    peers: Vec<PeerConfig>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct PeerConfig {
    pub node_id: String,
    pub public_key: String,
    pub incoming_token: String,
    pub outgoing_url: String,
    pub outgoing_token: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SyncRequest {
    events: Vec<Envelope>,
}

#[derive(Serialize)]
struct SyncResponse {
    events: Vec<Envelope>,
}

#[derive(Serialize)]
struct ApiError {
    error: &'static str,
}

impl PeerSync {
    pub fn load(path: &Path, store: Store) -> Result<Self> {
        let metadata = fs::symlink_metadata(path).context("inspect chat peer config")?;
        if !metadata.file_type().is_file() {
            bail!("chat peer config must be a regular file");
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            if metadata.permissions().mode() & 0o077 != 0 {
                bail!("chat peer config must not be group or world accessible");
            }
        }
        if metadata.len() > MAX_CONFIG_BYTES as u64 {
            bail!("chat peer config exceeds size limit");
        }
        let bytes = fs::read(path).context("read chat peer config")?;
        if bytes.len() > MAX_CONFIG_BYTES {
            bail!("chat peer config exceeds size limit");
        }
        let config: ConfigFile =
            serde_json::from_slice(&bytes).context("parse chat peer config")?;
        if config.peers.is_empty() {
            bail!("chat peer config must contain at least one peer");
        }

        let sync = Self::empty(store)?;
        sync.replace_peers(config.peers)?;
        Ok(sync)
    }

    pub fn empty(store: Store) -> Result<Self> {
        let identity = store.identity();
        let snapshot = PeerSnapshot {
            configs: Vec::new(),
            peers: Vec::new(),
            trusted_keys: BTreeMap::from([(identity.node_id, identity.public_key)]),
        };
        let client = distill_extra_ca::build_reqwest_client(|builder| {
            builder
                .timeout(PEER_TIMEOUT)
                .redirect(reqwest::redirect::Policy::none())
        })
        .context("build chat peer HTTP client")?;
        let status = SyncStatus::configured(0);
        let (changes, _) = tokio::sync::watch::channel(());
        Ok(Self {
            inner: Arc::new(PeerSyncInner {
                store,
                snapshot: Mutex::new(Arc::new(snapshot)),
                changes,
                client,
                status,
            }),
        })
    }

    pub(crate) fn replace_peers(&self, configs: Vec<PeerConfig>) -> Result<()> {
        let own_identity = self.inner.store.identity();
        let mut trusted_keys =
            BTreeMap::from([(own_identity.node_id.clone(), own_identity.public_key)]);
        let mut peers = Vec::with_capacity(configs.len());
        let mut peer_ids = std::collections::BTreeSet::new();
        let mut incoming_tokens = std::collections::BTreeSet::new();
        for peer in &configs {
            if peer.node_id == own_identity.node_id
                || !peer_ids.insert(peer.node_id.clone())
                || peer.node_id.len() != 64
                || !peer
                    .node_id
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            {
                bail!("chat peer config has invalid or duplicate peer identity");
            }
            let public_key = URL_SAFE_NO_PAD
                .decode(&peer.public_key)
                .context("decode chat peer public key")?;
            if URL_SAFE_NO_PAD.encode(&public_key) != peer.public_key || public_key.len() != 32 {
                bail!("chat peer config has invalid public key");
            }
            let identity_hash = digest(&SHA256, &public_key);
            if hex(identity_hash.as_ref()) != peer.node_id {
                bail!("chat peer public key does not match peer identity");
            }
            if !valid_token(&peer.incoming_token)
                || !valid_token(&peer.outgoing_token)
                || !incoming_tokens.insert(token_tag(&peer.incoming_token))
            {
                bail!("chat peer config has invalid or duplicate bearer token");
            }
            let outgoing_url = validate_peer_url(&peer.outgoing_url)?;
            trusted_keys.insert(peer.node_id.clone(), peer.public_key.clone());
            peers.push(Peer {
                incoming_token_tag: token_tag(&peer.incoming_token),
                outgoing_url,
                outgoing_token: peer.outgoing_token.clone(),
            });
        }
        let mut current = self
            .inner
            .snapshot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut status = self
            .inner
            .status
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        status.available = !peers.is_empty();
        status.peer_count = peers.len();
        status.last_attempt_unix_seconds = None;
        status.last_success_unix_seconds = None;
        status.last_error = None;
        *current = Arc::new(PeerSnapshot {
            configs,
            peers,
            trusted_keys,
        });
        self.inner.changes.send_replace(());
        Ok(())
    }

    pub(crate) fn peer_configs(&self) -> Vec<PeerConfig> {
        self.snapshot().configs.clone()
    }

    fn snapshot(&self) -> Arc<PeerSnapshot> {
        self.inner
            .snapshot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub fn status(&self) -> SyncStatus {
        self.inner.status.clone()
    }

    pub fn routes(&self) -> Router {
        Router::new()
            .route("/api/chat-sync", post(receive_snapshot))
            .layer(DefaultBodyLimit::max(MAX_SYNC_BODY_BYTES))
            .with_state(self.clone())
    }

    pub async fn run(&self, mut shutdown: tokio::sync::watch::Receiver<bool>) {
        let mut interval = tokio::time::interval(SYNC_INTERVAL);
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    let round = self.sync_round();
                    tokio::pin!(round);
                    tokio::select! {
                        _ = &mut round => {},
                        changed = shutdown.changed() => {
                            if changed.is_err() || *shutdown.borrow() { return; }
                        }
                    }
                }
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() { return; }
                }
            }
        }
    }

    async fn sync_round(&self) {
        let (snapshot, mut changes) = {
            let current = self
                .inner
                .snapshot
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            (current.clone(), self.inner.changes.subscribe())
        };
        if snapshot.peers.is_empty() {
            return;
        }
        let mut success = true;
        for peer in &snapshot.peers {
            let events = {
                let current = self
                    .inner
                    .snapshot
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if !Arc::ptr_eq(&current, &snapshot) {
                    return;
                }
                self.inner.store.export()
            };
            tokio::select! {
                biased;
                _ = changes.changed() => return,
                result = exchange(self, peer, events) => {
                    if result.is_err() {
                        success = false;
                        tracing::warn!("chat peer exchange failed");
                    }
                }
            }
        }
        let current = self
            .inner
            .snapshot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if Arc::ptr_eq(&current, &snapshot) {
            self.inner.status.record_round(success);
        }
    }

    fn authenticate(snapshot: &PeerSnapshot, headers: &HeaderMap) -> bool {
        let Some(value) = headers
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
        else {
            return false;
        };
        let Some(token) = value.strip_prefix("Bearer ") else {
            return false;
        };
        if !valid_token(token) {
            return false;
        }
        let supplied_key = HmacKey::new(HMAC_SHA256, token.as_bytes());
        let mut matched = 0_u8;
        for peer in &snapshot.peers {
            matched |= u8::from(
                hmac_verify(
                    &supplied_key,
                    b"distill-chat-sync-auth-v1",
                    &peer.incoming_token_tag,
                )
                .is_ok(),
            );
        }
        matched == 1
    }
}

async fn receive_snapshot(
    State(sync): State<PeerSync>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if headers.contains_key(ORIGIN) {
        return api_error(StatusCode::FORBIDDEN, "browser_origin_forbidden");
    }
    let snapshot = sync
        .inner
        .snapshot
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if !PeerSync::authenticate(&snapshot, &headers) {
        return api_error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    if body.len() > MAX_SYNC_BODY_BYTES {
        return api_error(StatusCode::PAYLOAD_TOO_LARGE, "snapshot_too_large");
    }
    let request: SyncRequest = match serde_json::from_slice(&body) {
        Ok(request) => request,
        Err(_) => return api_error(StatusCode::BAD_REQUEST, "invalid_snapshot"),
    };
    if sync
        .inner
        .store
        .import(request.events, &snapshot.trusted_keys)
        .is_err()
    {
        return api_error(StatusCode::BAD_REQUEST, "invalid_snapshot");
    }
    sync_response(&sync.inner.store)
}

fn sync_response(store: &Store) -> Response {
    let events = store.export();
    let bytes = match serde_json::to_vec(&SyncResponse { events }) {
        Ok(bytes) if bytes.len() <= MAX_SYNC_BODY_BYTES => bytes,
        _ => return api_error(StatusCode::PAYLOAD_TOO_LARGE, "snapshot_too_large"),
    };
    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        bytes,
    )
        .into_response()
}

async fn exchange(sync: &PeerSync, peer: &Peer, events: Vec<Envelope>) -> Result<()> {
    let request_body =
        serde_json::to_vec(&SyncRequest { events }).context("serialize chat peer snapshot")?;
    if request_body.len() > MAX_SYNC_BODY_BYTES {
        return Err(anyhow!("local chat snapshot exceeds peer body limit"));
    }
    let authorization = format!("Bearer {}", peer.outgoing_token);
    let response = sync
        .inner
        .client
        .post(peer.outgoing_url.clone())
        .header(AUTHORIZATION, authorization)
        .header(axum::http::header::CONTENT_TYPE, "application/json")
        .body(request_body)
        .send()
        .await
        .context("send chat peer snapshot")?;
    if !response.status().is_success() {
        return Err(anyhow!("chat peer returned an unsuccessful status"));
    }
    if response
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|length| length.to_str().ok())
        .and_then(|length| length.parse::<u64>().ok())
        .is_some_and(|length| length > MAX_SYNC_BODY_BYTES as u64)
    {
        return Err(anyhow!("chat peer response exceeds body limit"));
    }
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("read chat peer response")?;
        if bytes.len().saturating_add(chunk.len()) > MAX_SYNC_BODY_BYTES {
            return Err(anyhow!("chat peer response exceeds body limit"));
        }
        bytes.extend_from_slice(&chunk);
    }
    let response: SyncRequest =
        serde_json::from_slice(&bytes).context("parse chat peer response")?;
    sync.inner
        .store
        .import(response.events, &sync.snapshot().trusted_keys)
        .context("validate chat peer response")?;
    Ok(())
}

fn validate_peer_url(value: &str) -> Result<Url> {
    let url = Url::parse(value).context("parse chat peer URL")?;
    if !matches!(url.scheme(), "https" | "http")
        || url.username() != ""
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        bail!("chat peer URL must use HTTPS without credentials, query, or fragment");
    }
    if url.scheme() == "http" && !is_loopback_host(&url) {
        bail!("HTTP chat peer URL must use a loopback host");
    }
    Ok(url)
}

fn is_loopback_host(url: &Url) -> bool {
    match url.host() {
        Some(url::Host::Ipv4(address)) => address.is_loopback(),
        Some(url::Host::Ipv6(address)) => address.is_loopback(),
        Some(url::Host::Domain(name)) => name.eq_ignore_ascii_case("localhost"),
        None => false,
    }
}

fn valid_token(token: &str) -> bool {
    token.len() >= 32
        && token.len() <= 4096
        && token.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
}

fn token_tag(token: &str) -> [u8; 32] {
    hmac_sign(
        &HmacKey::new(HMAC_SHA256, token.as_bytes()),
        b"distill-chat-sync-auth-v1",
    )
    .as_ref()
    .try_into()
    .expect("HMAC-SHA-256 output length")
}

fn hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(output, "{byte:02x}").expect("write to string");
    }
    output
}

fn api_error(status: StatusCode, error: &'static str) -> Response {
    (status, axum::Json(ApiError { error })).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt as _;

    #[tokio::test]
    async fn replace_peers_updates_trust_atomically() {
        let local_dir = tempfile::tempdir().unwrap();
        let peer_dir = tempfile::tempdir().unwrap();
        let local = Store::open(&local_dir.path().join("store")).unwrap();
        let remote = Store::open(&peer_dir.path().join("store")).unwrap();
        let identity = remote.identity();
        let token = "incoming-secret-01234567890123456789";
        let peer = PeerConfig {
            node_id: identity.node_id,
            public_key: identity.public_key,
            incoming_token: token.into(),
            outgoing_url: "http://127.0.0.1/api/chat-sync".into(),
            outgoing_token: "outgoing-secret-01234567890123456789".into(),
        };
        let sync = PeerSync::empty(local).unwrap();
        let health = sync.status();
        sync.sync_round().await;
        assert!(!health.snapshot().available);
        assert_eq!(health.snapshot().last_success_unix_seconds, None);
        let receive = || {
            Request::builder()
                .method("POST")
                .uri("/api/chat-sync")
                .header(AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::from(r#"{"events":[]}"#))
                .unwrap()
        };
        assert_eq!(
            sync.routes().oneshot(receive()).await.unwrap().status(),
            StatusCode::UNAUTHORIZED
        );
        sync.replace_peers(vec![peer.clone()]).unwrap();
        assert_eq!(sync.peer_configs().len(), 1);
        assert!(health.snapshot().available);
        assert_eq!(health.snapshot().peer_count, 1);
        assert_eq!(
            sync.routes().oneshot(receive()).await.unwrap().status(),
            StatusCode::OK
        );
        let mut invalid = peer.clone();
        invalid.outgoing_url = "http://example.com/".into();
        assert!(sync.replace_peers(vec![invalid]).is_err());
        assert_eq!(sync.peer_configs()[0].node_id, peer.node_id);
        assert_eq!(
            sync.routes().oneshot(receive()).await.unwrap().status(),
            StatusCode::OK
        );
        sync.replace_peers(vec![]).unwrap();
        assert!(!health.snapshot().available);
        assert_eq!(health.snapshot().peer_count, 0);
        assert_eq!(
            sync.routes().oneshot(receive()).await.unwrap().status(),
            StatusCode::UNAUTHORIZED
        );
    }
    #[tokio::test]
    async fn removal_cancels_round_before_later_peer_receives_new_chat() {
        use crate::chat_store::ChatEvent;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tokio::sync::Notify;

        let local_dir = tempfile::tempdir().unwrap();
        let a_dir = tempfile::tempdir().unwrap();
        let b_dir = tempfile::tempdir().unwrap();
        let local = Store::open(&local_dir.path().join("store")).unwrap();
        let a = Store::open(&a_dir.path().join("store")).unwrap();
        let b = Store::open(&b_dir.path().join("store")).unwrap();
        let entered = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let count = Arc::new(AtomicUsize::new(0));
        let a_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let b_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let a_url = format!("http://{}/api/chat-sync", a_listener.local_addr().unwrap());
        let b_url = format!("http://{}/api/chat-sync", b_listener.local_addr().unwrap());
        let a_app = Router::new().route(
            "/api/chat-sync",
            post({
                let entered = entered.clone();
                let release = release.clone();
                move || async move {
                    entered.notify_one();
                    release.notified().await;
                    axum::Json(serde_json::json!({"events": []}))
                }
            }),
        );
        let b_app = Router::new().route(
            "/api/chat-sync",
            post({
                let count = count.clone();
                move || async move {
                    count.fetch_add(1, Ordering::SeqCst);
                    axum::Json(serde_json::json!({"events": []}))
                }
            }),
        );
        let a_server = tokio::spawn(async move { axum::serve(a_listener, a_app).await.unwrap() });
        let b_server = tokio::spawn(async move { axum::serve(b_listener, b_app).await.unwrap() });
        let config = |store: &Store, url, token: &str| {
            let identity = store.identity();
            PeerConfig {
                node_id: identity.node_id,
                public_key: identity.public_key,
                incoming_token: token.into(),
                outgoing_url: url,
                outgoing_token: "outgoing-secret-01234567890123456789".into(),
            }
        };
        let a_config = config(&a, a_url, "incoming-a-secret-01234567890123456789");
        let b_config = config(&b, b_url, "incoming-b-secret-01234567890123456789");
        let sync = PeerSync::empty(local.clone()).unwrap();
        sync.replace_peers(vec![a_config.clone(), b_config])
            .unwrap();
        let round_sync = sync.clone();
        let round = tokio::spawn(async move { round_sync.sync_round().await });
        tokio::time::timeout(Duration::from_secs(5), entered.notified())
            .await
            .unwrap();
        sync.replace_peers(vec![a_config]).unwrap();
        local
            .append(ChatEvent::Create {
                chat_id: "new".into(),
                title: "New".into(),
            })
            .unwrap();
        release.notify_one();
        tokio::time::timeout(Duration::from_secs(5), round)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 0);
        assert_eq!(sync.status().snapshot().last_success_unix_seconds, None);
        a_server.abort();
        b_server.abort();
    }
}
