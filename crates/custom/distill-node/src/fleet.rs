use axum::{Json, Router, routing::get};
use serde::Serialize;
use serde_json::Value;
use std::{
    net::{IpAddr, SocketAddr},
    process::Stdio,
    sync::Arc,
    time::Duration,
};
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

async fn discover() -> Result<Json<Vec<PeerHost>>, (axum::http::StatusCode, Json<Value>)> {
    discover_peers().await.map(Json).map_err(|_| {
        (
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "tailscale_discovery_unavailable"})),
        )
    })
}

async fn discover_peers() -> anyhow::Result<Vec<PeerHost>> {
    parse_peers(&tailscale_status().await?)
}

async fn tailscale_status() -> anyhow::Result<Vec<u8>> {
    #[expect(
        clippy::disallowed_methods,
        reason = "node-owned probe has a bounded timeout and kill_on_drop"
    )]
    let mut child = Command::new(
        std::env::var_os("POSTUDIO_TAILSCALE_BIN").unwrap_or_else(|| "tailscale".into()),
    )
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
    Ok(output)
}

/// For WSL hosts using the Windows Tailscale daemon, preserve the HTTPS name
/// while resolving known MagicDNS peers through the local daemon if DNS fails.
pub(crate) fn with_tailnet_dns(builder: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
    if std::env::var_os("POSTUDIO_TAILSCALE_BIN").is_some() {
        builder.dns_resolver(Arc::new(TailnetDns))
    } else {
        builder
    }
}

struct TailnetDns;

impl reqwest::dns::Resolve for TailnetDns {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        Box::pin(async move {
            let host = name.as_str();
            let system_error = match tokio::net::lookup_host((host.to_owned(), 0)).await {
                Ok(addresses) => return Ok(Box::new(addresses) as reqwest::dns::Addrs),
                Err(error) => error,
            };
            if host.ends_with(".ts.net") {
                if let Ok(output) = tailscale_status().await {
                    let addresses = peer_addresses(&output, host)?;
                    if !addresses.is_empty() {
                        return Ok(Box::new(addresses.into_iter()) as reqwest::dns::Addrs);
                    }
                }
            }
            Err(system_error.into())
        })
    }
}

fn peer_addresses(output: &[u8], host: &str) -> anyhow::Result<Vec<SocketAddr>> {
    let status: Value = serde_json::from_slice(output)?;
    let mut addresses = Vec::new();
    if let Some(peers) = status.get("Peer").and_then(Value::as_object) {
        for peer in peers.values() {
            if !peer
                .get("DNSName")
                .and_then(Value::as_str)
                .is_some_and(|name| name.trim_end_matches('.').eq_ignore_ascii_case(host))
            {
                continue;
            }
            if let Some(ips) = peer.get("TailscaleIPs").and_then(Value::as_array) {
                addresses.extend(
                    ips.iter()
                        .filter_map(Value::as_str)
                        .filter_map(|ip| ip.parse::<IpAddr>().ok())
                        .filter(|ip| match ip {
                            IpAddr::V4(ip) => {
                                ip.octets()[0] == 100 && (64..=127).contains(&ip.octets()[1])
                            }
                            IpAddr::V6(ip) => ip.segments()[..3] == [0xfd7a, 0x115c, 0xa1e0],
                        })
                        .map(|ip| SocketAddr::new(ip, 0)),
                );
            }
        }
    }
    Ok(addresses)
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
    fn fallback_resolves_only_matching_tailnet_addresses() {
        let output = br#"{"Peer":{"a":{"DNSName":"Peer.tailnet.ts.net.","TailscaleIPs":["100.64.1.2","fd7a:115c:a1e0::2","127.0.0.1","garbage"]},"b":{"DNSName":"other.tailnet.ts.net.","TailscaleIPs":["100.64.1.3"]}}}"#;
        let found = peer_addresses(output, "peer.tailnet.ts.net").unwrap();
        assert_eq!(
            found,
            vec![
                "100.64.1.2:0".parse().unwrap(),
                "[fd7a:115c:a1e0::2]:0".parse().unwrap()
            ]
        );
        assert!(
            peer_addresses(output, "unknown.tailnet.ts.net")
                .unwrap()
                .is_empty()
        );
    }

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
