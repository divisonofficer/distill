use axum::{Json, Router, routing::get};
use serde::Serialize;
use serde_json::Value;
use std::{process::Stdio, time::Duration};
use tokio::{io::AsyncReadExt, process::Command, time::timeout};

const MAX_STATUS_BYTES: usize = 1024 * 1024;

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct PeerHost {
    pub url: String,
    pub online: bool,
}

pub fn routes() -> Router {
    Router::new().route("/api/fleet", get(discover))
}

async fn discover() -> Json<Vec<PeerHost>> {
    Json(discover_peers().await.unwrap_or_default())
}

async fn discover_peers() -> anyhow::Result<Vec<PeerHost>> {
    #[expect(
        clippy::disallowed_methods,
        reason = "node-owned probe has a bounded timeout and kill_on_drop"
    )]
    let mut child = Command::new("tailscale")
        .args(["status", "--json"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("missing stdout"))?;
    let (output, status) = timeout(Duration::from_secs(2), async {
        let mut bytes = Vec::with_capacity(8192);
        stdout
            .take((MAX_STATUS_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .await?;
        if bytes.len() > MAX_STATUS_BYTES {
            child.kill().await?;
            anyhow::bail!("status output too large");
        }
        let status = child.wait().await?;
        anyhow::Ok((bytes, status))
    })
    .await??;
    if !status.success() {
        anyhow::bail!("tailscale status failed");
    }
    parse_peers(&output)
}

fn parse_peers(output: &[u8]) -> anyhow::Result<Vec<PeerHost>> {
    let value: Value = serde_json::from_slice(output)?;
    let mut peers = Vec::new();
    if let Some(nodes) = value.get("Peer").and_then(Value::as_object) {
        for node in nodes.values() {
            let Some(name) = node.get("DNSName").and_then(Value::as_str) else {
                continue;
            };
            let host = name.trim_end_matches('.');
            if host.is_empty()
                || !host
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
            {
                continue;
            }
            peers.push(PeerHost {
                url: format!("https://{host}"),
                online: node.get("Online").and_then(Value::as_bool).unwrap_or(false),
            });
        }
    }
    Ok(peers)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_only_dns_url_and_online_state() {
        let output = br#"{"Peer":{"a":{"DNSName":"peer.tailnet.ts.net.","Online":true,"TailscaleIPs":["100.64.1.2"],"UserID":3},"b":{"DNSName":"bad/name","Online":false}}}"#;
        let result = parse_peers(output).unwrap();
        assert_eq!(
            result,
            vec![PeerHost {
                url: "https://peer.tailnet.ts.net".into(),
                online: true
            }]
        );
    }
}
