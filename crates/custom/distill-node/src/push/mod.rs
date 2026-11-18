//! Web Push sender (CONTRACT §6): VAPID keys, subscription store, routes, fan-out.

mod encrypt;
mod vapid;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use anyhow::Context;
use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::Json;
use axum::routing::{get, post};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use vapid::Vapid;

const SUBSCRIPTIONS_FILE: &str = "push-subscriptions.json";
const SEND_TIMEOUT: Duration = Duration::from_secs(10);
const TTL_SECS: &str = "86400";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Subscription {
    endpoint: String,
    keys: Keys,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Keys {
    p256dh: String,
    auth: String,
}

#[derive(Deserialize)]
struct Unsubscribe {
    endpoint: String,
}

fn decode_b64url(s: &str) -> Option<Vec<u8>> {
    URL_SAFE_NO_PAD.decode(s.trim_end_matches('=')).ok()
}

impl Subscription {
    fn validate(&self) -> Result<(), &'static str> {
        let url = url::Url::parse(&self.endpoint).map_err(|_| "endpoint is not a URL")?;
        if url.scheme() != "https" {
            return Err("endpoint must be https");
        }
        match decode_b64url(&self.keys.p256dh) {
            Some(k) if k.len() == 65 && k.first() == Some(&0x04) => {}
            _ => return Err("p256dh must be an uncompressed P-256 point"),
        }
        match decode_b64url(&self.keys.auth) {
            Some(a) if a.len() == 16 => {}
            _ => return Err("auth must be 16 bytes"),
        }
        Ok(())
    }
}

pub struct Push {
    dir: PathBuf,
    vapid: Vapid,
    subscriptions: Mutex<Vec<Subscription>>,
    client: reqwest::Client,
}

impl Push {
    /// `dir` is the state dir (e.g. `~/.distill/postudio`). Created with 0700 if missing.
    pub fn load_or_create(dir: &Path) -> anyhow::Result<Self> {
        let vapid = Vapid::load_or_create(dir)?;
        let path = dir.join(SUBSCRIPTIONS_FILE);
        let subscriptions = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).context("parse push-subscriptions.json")?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e.into()),
        };
        // distill-node cannot depend on distill_extra_ca; push services are public HTTPS endpoints.
        #[allow(
            clippy::disallowed_methods,
            reason = "Web Push endpoints are public HTTPS; distill-node may not depend on distill_extra_ca"
        )]
        let client = reqwest::Client::builder().timeout(SEND_TIMEOUT).build()?;
        Ok(Self {
            dir: dir.to_path_buf(),
            vapid,
            subscriptions: Mutex::new(subscriptions),
            client,
        })
    }

    pub fn public_key_b64url(&self) -> String {
        self.vapid.public_key_b64url()
    }

    pub fn routes(self: Arc<Self>) -> Router {
        Router::new()
            .route("/api/push/key", get(key))
            .route("/api/push/subscribe", post(subscribe))
            .route("/api/push/unsubscribe", post(unsubscribe))
            .with_state(self)
    }

    /// Sends `payload` (JSON) to every subscription. 404/410 subscriptions are removed.
    pub async fn notify_all(&self, payload: &Value) {
        let body = match serde_json::to_vec(payload) {
            Ok(body) => body,
            Err(e) => {
                tracing::warn!(error = %e, "web push payload not serializable");
                return;
            }
        };
        let targets = self.lock().clone();
        let gone =
            futures_util::future::join_all(targets.iter().map(|s| self.send(s, &body))).await;
        let expired: Vec<String> = targets
            .iter()
            .zip(gone)
            .filter(|(_, gone)| *gone)
            .map(|(s, _)| s.endpoint.clone())
            .collect();
        if expired.is_empty() {
            return;
        }
        let mut subs = self.lock();
        subs.retain(|s| !expired.contains(&s.endpoint));
        if let Err(e) = self.persist(&subs) {
            tracing::warn!(error = %e, "save push subscriptions failed");
        }
    }

    /// Returns true when the push service says the subscription is gone.
    async fn send(&self, sub: &Subscription, body: &[u8]) -> bool {
        let Ok(url) = url::Url::parse(&sub.endpoint) else {
            tracing::warn!("web push skipped: stored endpoint is not a URL");
            return false;
        };
        let host = url.host_str().unwrap_or_default();
        let (Some(ua_public), Some(auth_secret)) = (
            decode_b64url(&sub.keys.p256dh),
            decode_b64url(&sub.keys.auth),
        ) else {
            tracing::warn!(%host, "web push skipped: stored keys are not base64url");
            return false;
        };
        let payload = match encrypt::encrypt(&ua_public, &auth_secret, body) {
            Ok(payload) => payload,
            Err(e) => {
                tracing::warn!(%host, error = %e, "web push encryption failed");
                return false;
            }
        };
        let authorization = match self
            .vapid
            .authorization(&url.origin().ascii_serialization())
        {
            Ok(value) => value,
            Err(e) => {
                tracing::warn!(%host, error = %e, "VAPID authorization failed");
                return false;
            }
        };
        let res = self
            .client
            .post(url.clone())
            .header(header::AUTHORIZATION, authorization)
            .header(header::CONTENT_ENCODING, "aes128gcm")
            .header("TTL", TTL_SECS)
            .header("Urgency", "high")
            .header(header::CONTENT_TYPE, "application/octet-stream")
            .body(payload)
            .send()
            .await;
        match res {
            Ok(resp) if resp.status().is_success() => false,
            Ok(resp) if matches!(resp.status(), StatusCode::NOT_FOUND | StatusCode::GONE) => true,
            Ok(resp) => {
                tracing::warn!(%host, status = %resp.status(), "web push rejected");
                false
            }
            Err(e) => {
                tracing::warn!(%host, error = %e.without_url(), "web push failed");
                false
            }
        }
    }

    fn lock(&self) -> MutexGuard<'_, Vec<Subscription>> {
        self.subscriptions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn upsert(&self, sub: Subscription) -> anyhow::Result<()> {
        let mut subs = self.lock();
        subs.retain(|s| s.endpoint != sub.endpoint);
        subs.push(sub);
        self.persist(&subs)
    }

    fn remove(&self, endpoint: &str) -> anyhow::Result<()> {
        let mut subs = self.lock();
        let before = subs.len();
        subs.retain(|s| s.endpoint != endpoint);
        if subs.len() != before {
            self.persist(&subs)?;
        }
        Ok(())
    }

    /// Write to a temp file, then rename over the store. Called with the lock held.
    fn persist(&self, subs: &[Subscription]) -> anyhow::Result<()> {
        let path = self.dir.join(SUBSCRIPTIONS_FILE);
        let tmp = self.dir.join(format!("{SUBSCRIPTIONS_FILE}.tmp"));
        // Subscriptions hold per-browser auth secrets: owner-only on unix.
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        std::io::Write::write_all(&mut opts.open(&tmp)?, &serde_json::to_vec(subs)?)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }
}

async fn key(State(push): State<Arc<Push>>) -> Json<Value> {
    Json(json!({ "publicKey": push.public_key_b64url() }))
}

async fn subscribe(State(push): State<Arc<Push>>, body: Bytes) -> StatusCode {
    let Ok(sub) = serde_json::from_slice::<Subscription>(&body) else {
        return StatusCode::BAD_REQUEST;
    };
    if sub.validate().is_err() {
        return StatusCode::BAD_REQUEST;
    }
    match push.upsert(sub) {
        Ok(()) => StatusCode::NO_CONTENT,
        Err(e) => {
            tracing::warn!(error = %e, "save push subscription failed");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

async fn unsubscribe(State(push): State<Arc<Push>>, body: Bytes) -> StatusCode {
    let Ok(req) = serde_json::from_slice::<Unsubscribe>(&body) else {
        return StatusCode::BAD_REQUEST;
    };
    match push.remove(&req.endpoint) {
        Ok(()) => StatusCode::NO_CONTENT,
        Err(e) => {
            tracing::warn!(error = %e, "save push subscriptions failed");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}
