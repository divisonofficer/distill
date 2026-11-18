//! Listener setup: Unix socket in a 0700 directory on Unix, loopback TCP elsewhere.
//! Public API: plan/research-base/CONTRACT.md §5.

use std::net::SocketAddr;
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{Context, bail};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListenAddr {
    Tcp(SocketAddr),
    #[cfg(unix)]
    Unix(PathBuf),
}

impl FromStr for ListenAddr {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> anyhow::Result<Self> {
        if let Some(addr) = s.strip_prefix("tcp:") {
            return Ok(Self::Tcp(addr.parse().context("invalid tcp address")?));
        }
        #[cfg(unix)]
        if let Some(path) = s.strip_prefix("unix:") {
            if path.is_empty() {
                bail!("unix socket path is empty");
            }
            return Ok(Self::Unix(PathBuf::from(path)));
        }
        bail!("listen address must be tcp:<addr:port> or unix:<path>, got {s:?}")
    }
}

/// Trusted listener: `~/.distill/postudio/node.sock` on Unix, loopback TCP 7410 elsewhere.
pub fn default_trusted() -> ListenAddr {
    #[cfg(unix)]
    {
        postudio_socket("node.sock")
    }
    #[cfg(not(unix))]
    {
        ListenAddr::Tcp(SocketAddr::from(([127, 0, 0, 1], 7410)))
    }
}

/// Untrusted listener: `~/.distill/postudio/untrusted.sock` on Unix, loopback TCP 7411 elsewhere.
pub fn default_untrusted() -> ListenAddr {
    #[cfg(unix)]
    {
        postudio_socket("untrusted.sock")
    }
    #[cfg(not(unix))]
    {
        ListenAddr::Tcp(SocketAddr::from(([127, 0, 0, 1], 7411)))
    }
}

/// Empty path when the home directory is unset; `serve` rejects it.
#[cfg(unix)]
fn postudio_socket(name: &str) -> ListenAddr {
    match std::env::var_os("HOME") {
        Some(home) => ListenAddr::Unix(PathBuf::from(home).join(".distill/postudio").join(name)),
        None => ListenAddr::Unix(PathBuf::new()),
    }
}

/// Bind `addr` and serve `app` until the listener fails.
pub async fn serve(addr: ListenAddr, app: axum::Router) -> anyhow::Result<()> {
    match addr {
        ListenAddr::Tcp(addr) => {
            if !addr.ip().is_loopback() {
                bail!("refusing to listen on non-loopback address {addr}");
            }
            let listener = tokio::net::TcpListener::bind(addr)
                .await
                .with_context(|| format!("bind {addr}"))?;
            tracing::info!(%addr, "listening");
            axum::serve(listener, app).await?;
        }
        #[cfg(unix)]
        ListenAddr::Unix(path) => {
            if path.as_os_str().is_empty() {
                bail!("socket path is empty (HOME unset?)");
            }
            prepare_socket_dir(&path)?;
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    return Err(e).with_context(|| format!("remove stale {}", path.display()));
                }
            }
            let listener = tokio::net::UnixListener::bind(&path)
                .with_context(|| format!("bind {}", path.display()))?;
            tracing::info!(path = %path.display(), "listening");
            axum::serve(listener, app).await?;
        }
    }
    Ok(())
}

/// Create the socket's parent directory as 0700, tightening it if it already exists.
#[cfg(unix)]
fn prepare_socket_dir(socket: &Path) -> anyhow::Result<()> {
    let Some(dir) = socket.parent().filter(|p| !p.as_os_str().is_empty()) else {
        return Ok(());
    };
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .with_context(|| format!("create {}", dir.display()))?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
        .with_context(|| format!("chmod 0700 {}", dir.display()))?;
    Ok(())
}
