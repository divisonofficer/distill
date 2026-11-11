use clap::Parser;
use distill_node::NodeConfig;

/// Postudio node: per-host gateway in front of the local Distill agent.
#[derive(Debug, Parser)]
#[command(name = "postudio-node", version = distill_version::VERSION)]
struct Args {
    /// Listen address. Keep it on loopback; `tailscale serve` exposes it over HTTPS.
    #[arg(long, default_value = "127.0.0.1:7410")]
    bind: std::net::SocketAddr,
    /// Tailscale login allowed to use the API (e.g. you@github). Required unless --insecure-no-owner.
    #[arg(long, env = "POSTUDIO_OWNER")]
    owner: Option<String>,
    /// Skip the owner check. Local development only.
    #[arg(long, conflicts_with = "owner")]
    insecure_no_owner: bool,
    /// Local Distill agent WebSocket URL (`distill agent serve`).
    #[arg(long, default_value = "ws://127.0.0.1:2419/ws")]
    agent_url: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    let args = Args::parse();
    if args.owner.is_none() && !args.insecure_no_owner {
        anyhow::bail!(
            "--owner <tailscale login> is required (or --insecure-no-owner for local dev)"
        );
    }
    let listener = tokio::net::TcpListener::bind(args.bind).await?;
    let config = NodeConfig {
        owner_login: args.owner,
        agent_url: args.agent_url,
    };
    distill_node::serve(listener, config, async {
        let _ = tokio::signal::ctrl_c().await;
    })
    .await
}
