//! VAPID key pair (ES256 / P-256) and the RFC 8292 `Authorization` header.

use std::io::{ErrorKind, Write};
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, anyhow};
use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use ring::rand::SystemRandom;
use ring::signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair};
use serde::{Deserialize, Serialize};

const KEY_FILE: &str = "vapid.json";
const JWT_TTL: Duration = Duration::from_secs(12 * 60 * 60);
const SUBJECT: &str = "mailto:me@example.com";

#[derive(Serialize, Deserialize)]
struct KeyFile {
    pkcs8: String,
}

pub struct Vapid {
    key: EcdsaKeyPair,
    public: Vec<u8>,
    rng: SystemRandom,
}

impl Vapid {
    pub fn load_or_create(dir: &Path) -> anyhow::Result<Self> {
        create_dir(dir).with_context(|| format!("create {}", dir.display()))?;
        let rng = SystemRandom::new();
        let path = dir.join(KEY_FILE);
        let pkcs8 = match std::fs::read(&path) {
            Ok(bytes) => {
                let file: KeyFile = serde_json::from_slice(&bytes).context("parse vapid.json")?;
                STANDARD.decode(file.pkcs8).context("decode VAPID pkcs8")?
            }
            Err(e) if e.kind() == ErrorKind::NotFound => {
                let doc = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng)
                    .map_err(|_| anyhow!("generate VAPID key"))?;
                let file = KeyFile {
                    pkcs8: STANDARD.encode(doc.as_ref()),
                };
                write_private(&path, &serde_json::to_vec(&file)?)?;
                doc.as_ref().to_vec()
            }
            Err(e) => return Err(e.into()),
        };
        let key = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &pkcs8, &rng)
            .map_err(|_| anyhow!("load VAPID key"))?;
        let public = key.public_key().as_ref().to_vec();
        Ok(Self { key, public, rng })
    }

    pub fn public_key_b64url(&self) -> String {
        URL_SAFE_NO_PAD.encode(&self.public)
    }

    /// `Authorization` value for a push to `audience` (the endpoint's scheme://host[:port]).
    pub fn authorization(&self, audience: &str) -> anyhow::Result<String> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        let header = URL_SAFE_NO_PAD.encode(br#"{"typ":"JWT","alg":"ES256"}"#);
        let claims = json_claims(audience, now + JWT_TTL.as_secs());
        let input = format!(
            "{header}.{}",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims)?)
        );
        let sig = self
            .key
            .sign(&self.rng, input.as_bytes())
            .map_err(|_| anyhow!("sign VAPID JWT"))?;
        Ok(format!(
            "vapid t={input}.{}, k={}",
            URL_SAFE_NO_PAD.encode(sig.as_ref()),
            self.public_key_b64url()
        ))
    }
}

fn json_claims(audience: &str, exp: u64) -> serde_json::Value {
    serde_json::json!({ "aud": audience, "exp": exp, "sub": SUBJECT })
}

fn create_dir(dir: &Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir)
}

fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut file = opts.open(path)?;
    file.write_all(bytes)
}
