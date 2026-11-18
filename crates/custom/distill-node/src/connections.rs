use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow, bail};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode, header::ORIGIN},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use futures_util::StreamExt as _;
use ring::{
    hmac,
    rand::{SecureRandom, SystemRandom},
};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::chat_store::Store;
use crate::chat_sync::{PeerConfig, PeerSync};

const LIMIT: usize = 16 * 1024;
const MAX_PEERS: usize = 16;
const LIFETIME: u64 = 600;

#[derive(Clone)]
pub struct Connections {
    inner: Arc<Inner>,
}
struct Inner {
    store: Store,
    sync: PeerSync,
    path: PathBuf,
    allowed: Vec<String>,
    forbidden: String,
    client: reqwest::Client,
    state: Mutex<ConnectionState>,
}
struct ConnectionState {
    invitations: Vec<Pending>,
}
struct Pending {
    token: String,
    expires_at: u64,
    origin: String,
    incoming_token: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Invitation {
    version: u8,
    origin: String,
    public_key: String,
    node_id: String,
    token: String,
    expires_at: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OriginRequest {
    origin: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct JoinRequest {
    invitation: String,
    origin: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RemoveRequest {
    node_id: String,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AcceptRequest {
    token: String,
    origin: String,
    node_id: String,
    public_key: String,
    incoming_token: String,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AcceptResponse {
    origin: String,
    node_id: String,
    public_key: String,
    incoming_token: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PeerFile {
    peers: Vec<PeerConfig>,
}
#[derive(Serialize)]
struct ApiError {
    error: &'static str,
}
fn error(status: StatusCode, message: &'static str) -> Response {
    (status, Json(ApiError { error: message })).into_response()
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn random_token() -> Result<String> {
    let mut bytes = [0; 32];
    SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| anyhow!("random generation failed"))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}
fn token_matches(expected: &str, supplied: &str) -> bool {
    let tag = hmac::sign(
        &hmac::Key::new(hmac::HMAC_SHA256, expected.as_bytes()),
        b"distill-chat-pair-v1",
    );
    hmac::verify(
        &hmac::Key::new(hmac::HMAC_SHA256, supplied.as_bytes()),
        b"distill-chat-pair-v1",
        tag.as_ref(),
    )
    .is_ok()
}
fn origin(value: &str, forbidden: &str) -> bool {
    let Ok(url) = Url::parse(value) else {
        return false;
    };
    let loopback = match url.host() {
        Some(url::Host::Ipv4(a)) => a.is_loopback(),
        Some(url::Host::Ipv6(a)) => a.is_loopback(),
        Some(url::Host::Domain(a)) => a == "localhost",
        None => false,
    };
    (url.scheme() == "https" || (url.scheme() == "http" && loopback))
        && url.username().is_empty()
        && url.password().is_none()
        && url.path() == "/"
        && url.query().is_none()
        && url.fragment().is_none()
        && url.port_or_known_default() != Some(8443)
        && url.origin().ascii_serialization() == value
        && value != forbidden
}
fn peer_origin(peer: &PeerConfig) -> Option<String> {
    Url::parse(&peer.outgoing_url)
        .ok()
        .map(|u| u.origin().ascii_serialization())
}
fn sync_url(origin: &str) -> String {
    format!("{origin}/api/chat-sync")
}

impl Connections {
    // A managed file is an authoritative snapshot; without one, legacy peers remain active.
    pub fn load_or_create(
        state_dir: &Path,
        store: Store,
        sync: PeerSync,
        allowed_origins: Vec<String>,
        forbidden_origin: String,
    ) -> Result<Self> {
        let path = state_dir.join("managed-chat-peers.json");
        if path.exists() {
            let metadata = fs::symlink_metadata(&path)?;
            if !metadata.file_type().is_file() || metadata.len() > LIMIT as u64 {
                bail!("invalid managed peer file");
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                if metadata.permissions().mode() & 0o077 != 0 {
                    bail!("invalid managed peer permissions");
                }
            }
            let bytes = fs::read(&path)?;
            if bytes.len() > LIMIT {
                bail!("invalid managed peer file");
            }
            let file: PeerFile = serde_json::from_slice(&bytes)?;
            if file.peers.len() > MAX_PEERS
                || file
                    .peers
                    .iter()
                    .any(|p| peer_origin(p).is_none_or(|o| !origin(&o, &forbidden_origin)))
            {
                bail!("invalid managed peers");
            }
            sync.replace_peers(file.peers)?;
        }
        let client = distill_extra_ca::build_reqwest_client(|b| {
            b.timeout(Duration::from_secs(10))
                .redirect(reqwest::redirect::Policy::none())
        })?;
        Ok(Self {
            inner: Arc::new(Inner {
                store,
                sync,
                path,
                allowed: allowed_origins,
                forbidden: forbidden_origin,
                client,
                state: Mutex::new(ConnectionState {
                    invitations: Vec::new(),
                }),
            }),
        })
    }
    pub fn owner_routes(&self) -> Router {
        Router::new()
            .route("/api/connections/invite", post(invite))
            .route("/api/connections/join", post(join))
            .route("/api/connections", get(list))
            .route("/api/connections/remove", post(remove))
            .layer(DefaultBodyLimit::max(LIMIT))
            .with_state(self.clone())
    }
    pub fn public_routes(&self) -> Router {
        Router::new()
            .route("/api/chat-pair/accept", post(accept))
            .layer(DefaultBodyLimit::max(LIMIT))
            .with_state(self.clone())
    }
    pub fn allows_origin(&self, value: &str) -> bool {
        origin(value, &self.inner.forbidden)
            && self
                .inner
                .sync
                .peer_configs()
                .iter()
                .any(|p| peer_origin(p).as_deref() == Some(value))
    }
    fn owner_origin(&self, headers: &HeaderMap, value: &str) -> bool {
        origin(value, &self.inner.forbidden)
            && self.inner.allowed.iter().any(|o| o == value)
            && headers.get_all(ORIGIN).iter().count() == 1
            && headers.get(ORIGIN).and_then(|v| v.to_str().ok()) == Some(value)
    }
    fn persist(&self, peers: Vec<PeerConfig>) -> Result<()> {
        if peers.len() > MAX_PEERS {
            bail!("too many peers");
        }
        let test = PeerSync::empty(self.inner.store.clone())?;
        test.replace_peers(peers.clone())?;
        let bytes = serde_json::to_vec(&PeerFile {
            peers: peers.clone(),
        })?;
        if bytes.len() > LIMIT {
            bail!("peer file too large");
        }
        let dir = self
            .inner
            .path
            .parent()
            .ok_or_else(|| anyhow!("invalid state directory"))?;
        fs::create_dir_all(dir)?;
        let temp = dir.join(format!(".managed-chat-peers.{}.tmp", random_token()?));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let result = (|| -> Result<()> {
            let mut file = options.open(&temp)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            fs::rename(&temp, &self.inner.path)?;
            #[cfg(unix)]
            fs::File::open(dir)?.sync_all()?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        result?;
        self.inner.sync.replace_peers(peers)?;
        Ok(())
    }
}
async fn invite(State(c): State<Connections>, headers: HeaderMap, body: Bytes) -> Response {
    let Ok(req) = serde_json::from_slice::<OriginRequest>(&body) else {
        return error(StatusCode::BAD_REQUEST, "invalid_request");
    };
    if !c.owner_origin(&headers, &req.origin) {
        return error(StatusCode::FORBIDDEN, "invalid_origin");
    }
    let Ok(token) = random_token() else {
        return error(StatusCode::INTERNAL_SERVER_ERROR, "pairing_failed");
    };
    let Ok(incoming_token) = random_token() else {
        return error(StatusCode::INTERNAL_SERVER_ERROR, "pairing_failed");
    };
    let expires_at = now().saturating_add(LIFETIME);
    let identity = c.inner.store.identity();
    let invitation = Invitation {
        version: 1,
        origin: req.origin.clone(),
        public_key: identity.public_key,
        node_id: identity.node_id,
        token: token.clone(),
        expires_at,
    };
    let Ok(bytes) = serde_json::to_vec(&invitation) else {
        return error(StatusCode::INTERNAL_SERVER_ERROR, "pairing_failed");
    };
    let mut state = c.inner.state.lock().unwrap_or_else(|e| e.into_inner());
    state.invitations.retain(|i| i.expires_at > now());
    if state.invitations.len() >= 8 {
        return error(StatusCode::CONFLICT, "invitation_limit");
    }
    state.invitations.push(Pending {
        token,
        expires_at,
        origin: req.origin,
        incoming_token,
    });
    Json(serde_json::json!({"invitation": URL_SAFE_NO_PAD.encode(bytes), "expiresAt": expires_at}))
        .into_response()
}
async fn accept(State(c): State<Connections>, headers: HeaderMap, body: Bytes) -> Response {
    if headers.contains_key(ORIGIN) {
        return error(StatusCode::FORBIDDEN, "browser_origin_forbidden");
    }
    let Ok(req) = serde_json::from_slice::<AcceptRequest>(&body) else {
        return error(StatusCode::BAD_REQUEST, "invalid_request");
    };
    if !origin(&req.origin, &c.inner.forbidden) {
        return error(StatusCode::BAD_REQUEST, "invalid_origin");
    }
    let mut state = c.inner.state.lock().unwrap_or_else(|e| e.into_inner());
    let Some(index) = state
        .invitations
        .iter()
        .position(|i| i.expires_at > now() && token_matches(&i.token, &req.token))
    else {
        return error(StatusCode::UNAUTHORIZED, "invalid_invitation");
    };
    let pending = &state.invitations[index];
    let mut peers = c.inner.sync.peer_configs();
    if peers.len() >= MAX_PEERS || peers.iter().any(|p| p.node_id == req.node_id) {
        return error(StatusCode::CONFLICT, "peer_conflict");
    }
    peers.push(PeerConfig {
        node_id: req.node_id,
        public_key: req.public_key,
        incoming_token: pending.incoming_token.clone(),
        outgoing_url: sync_url(&req.origin),
        outgoing_token: req.incoming_token,
    });
    if c.persist(peers).is_err() {
        return error(StatusCode::BAD_REQUEST, "pairing_failed");
    }
    let pending = state.invitations.remove(index);
    let identity = c.inner.store.identity();
    Json(AcceptResponse {
        origin: pending.origin,
        node_id: identity.node_id,
        public_key: identity.public_key,
        incoming_token: pending.incoming_token,
    })
    .into_response()
}
async fn join(State(c): State<Connections>, headers: HeaderMap, body: Bytes) -> Response {
    let Ok(req) = serde_json::from_slice::<JoinRequest>(&body) else {
        return error(StatusCode::BAD_REQUEST, "invalid_request");
    };
    if !c.owner_origin(&headers, &req.origin) {
        return error(StatusCode::FORBIDDEN, "invalid_origin");
    }
    let Ok(raw) = URL_SAFE_NO_PAD.decode(&req.invitation) else {
        return error(StatusCode::BAD_REQUEST, "invalid_invitation");
    };
    let Ok(inv) = serde_json::from_slice::<Invitation>(&raw) else {
        return error(StatusCode::BAD_REQUEST, "invalid_invitation");
    };
    if inv.version != 1
        || inv.expires_at <= now()
        || inv.expires_at > now().saturating_add(LIFETIME)
        || !origin(&inv.origin, &c.inner.forbidden)
        || URL_SAFE_NO_PAD.decode(&inv.token).map_or(true, |b| {
            b.len() != 32 || URL_SAFE_NO_PAD.encode(b) != inv.token
        })
    {
        return error(StatusCode::BAD_REQUEST, "invalid_invitation");
    }
    let Ok(incoming_token) = random_token() else {
        return error(StatusCode::INTERNAL_SERVER_ERROR, "pairing_failed");
    };
    {
        let _state = c.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        let peers = c.inner.sync.peer_configs();
        if peers.len() >= MAX_PEERS || peers.iter().any(|p| p.node_id == inv.node_id) {
            return error(StatusCode::CONFLICT, "peer_conflict");
        }
    }
    let candidate = PeerConfig {
        node_id: inv.node_id.clone(),
        public_key: inv.public_key.clone(),
        incoming_token: incoming_token.clone(),
        outgoing_url: sync_url(&inv.origin),
        outgoing_token: incoming_token.clone(),
    };
    if PeerSync::empty(c.inner.store.clone())
        .and_then(|sync| sync.replace_peers(vec![candidate]))
        .is_err()
    {
        return error(StatusCode::BAD_REQUEST, "invalid_invitation");
    }
    let identity = c.inner.store.identity();
    let request = AcceptRequest {
        token: inv.token,
        origin: req.origin,
        node_id: identity.node_id,
        public_key: identity.public_key,
        incoming_token: incoming_token.clone(),
    };
    let Ok(response) = c
        .inner
        .client
        .post(format!("{}/api/chat-pair/accept", inv.origin))
        .json(&request)
        .send()
        .await
    else {
        return error(StatusCode::BAD_GATEWAY, "pairing_incomplete");
    };
    if !response.status().is_success() {
        return error(StatusCode::BAD_GATEWAY, "pairing_incomplete");
    }
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let Ok(chunk) = chunk else {
            return error(StatusCode::BAD_GATEWAY, "pairing_incomplete");
        };
        if bytes.len().saturating_add(chunk.len()) > LIMIT {
            return error(StatusCode::BAD_GATEWAY, "pairing_incomplete");
        }
        bytes.extend_from_slice(&chunk);
    }
    let Ok(remote) = serde_json::from_slice::<AcceptResponse>(&bytes) else {
        return error(StatusCode::BAD_GATEWAY, "pairing_incomplete");
    };
    if remote.node_id != inv.node_id
        || remote.public_key != inv.public_key
        || remote.origin != inv.origin
    {
        return error(StatusCode::BAD_GATEWAY, "pairing_incomplete");
    }
    let _state = c.inner.state.lock().unwrap_or_else(|e| e.into_inner());
    let mut peers = c.inner.sync.peer_configs();
    if peers.len() >= MAX_PEERS || peers.iter().any(|p| p.node_id == remote.node_id) {
        return error(StatusCode::CONFLICT, "pairing_incomplete");
    }
    peers.push(PeerConfig {
        node_id: remote.node_id.clone(),
        public_key: remote.public_key,
        incoming_token,
        outgoing_url: sync_url(&remote.origin),
        outgoing_token: remote.incoming_token,
    });
    if c.persist(peers).is_err() {
        return error(StatusCode::INTERNAL_SERVER_ERROR, "pairing_incomplete");
    }
    Json(serde_json::json!({"nodeId": remote.node_id, "origin": remote.origin})).into_response()
}
async fn list(State(c): State<Connections>) -> Response {
    let _state = c.inner.state.lock().unwrap_or_else(|e| e.into_inner());
    let peers: Vec<_> = c
        .inner
        .sync
        .peer_configs()
        .iter()
        .filter_map(|p| {
            peer_origin(p).map(|origin| serde_json::json!({"nodeId": p.node_id, "origin": origin}))
        })
        .collect();
    let identity = c.inner.store.identity();
    Json(serde_json::json!({"identity": {"nodeId": identity.node_id, "publicKey": identity.public_key}, "peers": peers, "sync": c.inner.sync.status().snapshot()})).into_response()
}
async fn remove(State(c): State<Connections>, body: Bytes) -> Response {
    let Ok(req) = serde_json::from_slice::<RemoveRequest>(&body) else {
        return error(StatusCode::BAD_REQUEST, "invalid_request");
    };
    let _state = c.inner.state.lock().unwrap_or_else(|e| e.into_inner());
    let mut peers = c.inner.sync.peer_configs();
    if !peers.iter().any(|p| p.node_id == req.node_id) {
        return error(StatusCode::NOT_FOUND, "peer_not_found");
    }
    peers.retain(|p| p.node_id != req.node_id);
    if c.persist(peers).is_err() {
        return error(StatusCode::INTERNAL_SERVER_ERROR, "removal_failed");
    }
    Json(serde_json::json!({"ok": true})).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn origins_are_canonical_and_tokens_match() {
        assert!(origin("https://example.com", "https://bad.example"));
        assert!(origin("http://127.0.0.1:1234", ""));
        for value in [
            "http://example.com",
            "https://example.com/",
            "https://user@example.com",
            "https://example.com:8443",
            "https://example.com?q=1",
            "https://bad.example",
        ] {
            assert!(!origin(value, "https://bad.example"), "{value}");
        }
        assert!(token_matches("secret", "secret"));
        assert!(!token_matches("secret", "other"));
    }
    #[tokio::test]
    async fn invitation_replay_expiry_and_removal() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("local")).unwrap();
        let remote = Store::open(&dir.path().join("remote")).unwrap();
        let sync = PeerSync::empty(store.clone()).unwrap();
        let c = Connections::load_or_create(dir.path(), store, sync.clone(), vec![], String::new())
            .unwrap();
        let identity = remote.identity();
        let make_body = |token: &str| {
            Bytes::from(
                serde_json::to_vec(&AcceptRequest {
                    token: token.to_owned(),
                    origin: "https://remote.example".into(),
                    node_id: identity.node_id.clone(),
                    public_key: identity.public_key.clone(),
                    incoming_token: random_token().unwrap(),
                })
                .unwrap(),
            )
        };
        let token = random_token().unwrap();
        c.inner.state.lock().unwrap().invitations.push(Pending {
            token: token.clone(),
            expires_at: now() + 60,
            origin: "https://local.example".into(),
            incoming_token: random_token().unwrap(),
        });
        assert_eq!(
            accept(State(c.clone()), HeaderMap::new(), make_body(&token))
                .await
                .status(),
            StatusCode::OK
        );
        assert_eq!(
            accept(State(c.clone()), HeaderMap::new(), make_body(&token))
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
        assert!(c.allows_origin("https://remote.example"));
        let body = Bytes::from(
            serde_json::to_vec(&serde_json::json!({"nodeId": identity.node_id})).unwrap(),
        );
        assert_eq!(
            remove(State(c.clone()), body).await.status(),
            StatusCode::OK
        );
        assert!(!c.allows_origin("https://remote.example"));
        let reloaded = Connections::load_or_create(
            dir.path(),
            c.inner.store.clone(),
            sync,
            vec![],
            String::new(),
        )
        .unwrap();
        assert!(!reloaded.allows_origin("https://remote.example"));
        c.inner.state.lock().unwrap().invitations.push(Pending {
            token: token.clone(),
            expires_at: now().saturating_sub(1),
            origin: "https://local.example".into(),
            incoming_token: random_token().unwrap(),
        });
        assert_eq!(
            accept(State(c), HeaderMap::new(), make_body(&token))
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }
}
