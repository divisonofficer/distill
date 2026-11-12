//! Local listening TCP port discovery for the Studio.

use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncReadExt;

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::get;
use serde::Serialize;
use tokio::process::Command;

#[derive(Clone)]
struct Config {
    blocked: Vec<u16>,
}

#[derive(Serialize, Debug, PartialEq, Eq)]
struct Port {
    port: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    process: Option<String>,
}

#[derive(Serialize)]
struct PortList {
    ports: Vec<Port>,
}

pub fn routes(blocked_ports: Vec<u16>) -> Router {
    Router::new()
        .route("/api/ports", get(list))
        .with_state(Config {
            blocked: blocked_ports,
        })
}

async fn list(State(config): State<Config>) -> (StatusCode, axum::Json<serde_json::Value>) {
    #[cfg(windows)]
    {
        let _ = config;
        return (
            StatusCode::NOT_IMPLEMENTED,
            axum::Json(serde_json::json!({"error":"port discovery is unsupported on Windows"})),
        );
    }
    #[cfg(not(windows))]
    {
        match discover(&config.blocked).await {
            Ok(ports) => (
                StatusCode::OK,
                axum::Json(serde_json::json!(PortList { ports })),
            ),
            Err(error) => (
                StatusCode::SERVICE_UNAVAILABLE,
                axum::Json(serde_json::json!({"error": error})),
            ),
        }
    }
}

#[cfg(not(windows))]
async fn discover(blocked: &[u16]) -> Result<Vec<Port>, String> {
    #[expect(
        clippy::disallowed_methods,
        reason = "This fixed-argument lsof probe has a 3-second timeout and kill_on_drop; it is not a long-lived session process."
    )]
    let mut child = Command::new("lsof")
        .args(["-nP", "-iTCP", "-sTCP:LISTEN", "-Fpcn"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| "port discovery requires lsof".to_owned())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "could not read lsof output".to_owned())?;
    let work = async {
        let mut bytes = Vec::new();
        stdout
            .take(1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| "could not read lsof output".to_owned())?;
        let status = child
            .wait()
            .await
            .map_err(|_| "port discovery failed".to_owned())?;
        Ok::<_, String>((status, bytes))
    };
    let (status, bytes) = tokio::time::timeout(Duration::from_secs(3), work)
        .await
        .map_err(|_| "port discovery timed out".to_owned())??;
    if !(status.success() || status.code() == Some(1) && bytes.is_empty()) {
        return Err("lsof could not inspect listening ports".into());
    }
    if bytes.len() > 1024 * 1024 {
        return Err("port discovery output exceeded limit".into());
    }
    Ok(parse_lsof(&bytes, blocked))
}

fn parse_lsof(output: &[u8], blocked: &[u16]) -> Vec<Port> {
    let text = String::from_utf8_lossy(output);
    let mut process: Option<String> = None;
    let mut ports = std::collections::BTreeMap::<u16, Option<String>>::new();
    for line in text.lines() {
        if let Some(name) = line.strip_prefix('c') {
            process = Some(name.chars().take(120).collect());
        }
        let Some(name) = line.strip_prefix('n') else {
            continue;
        };
        let Some((host, port)) = name.rsplit_once(':') else {
            continue;
        };
        let Ok(port) = port.parse::<u16>() else {
            continue;
        };
        if port == 0 || blocked.contains(&port) || !connectable_host(host) {
            continue;
        }
        ports.entry(port).or_insert_with(|| process.clone());
    }
    ports
        .into_iter()
        .map(|(port, process)| Port { port, process })
        .collect()
}

fn connectable_host(host: &str) -> bool {
    if host == "*" || host == "0.0.0.0" || host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    host.parse::<std::net::Ipv4Addr>()
        .is_ok_and(|ip| ip.is_loopback() || ip.is_unspecified())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_listeners_filters_unroutable_and_blocked() {
        let data = b"p123\ncpython\nf3\nn127.0.0.1:6006\nn*:6007\nn192.0.2.4:6008\nn[::1]:6009\nn0.0.0.0:7410\n";
        assert_eq!(
            parse_lsof(data, &[7410]),
            vec![
                Port {
                    port: 6006,
                    process: Some("python".into())
                },
                Port {
                    port: 6007,
                    process: Some("python".into())
                },
            ]
        );
    }
}
