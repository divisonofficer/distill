use std::path::PathBuf;

use anyhow::Context as _;
use clap::Parser;
use distill_node::agent::AgentConfig;
use distill_node::listen::{self, ListenAddr};
use distill_node::{NodeConfig, run};

/// Postudio node: per-host gateway in front of the local Distill agent.
#[derive(Debug, Parser)]
#[command(name = "postudio-node", version = distill_version::VERSION)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, clap::Subcommand)]
enum Command {
    /// Run the node until a listener fails.
    Serve(Serve),
}

#[derive(Debug, clap::Args)]
struct Serve {
    /// Tailscale login allowed to use the API (e.g. me@example.com).
    #[arg(long, env = "POSTUDIO_OWNER")]
    owner: String,
    /// Trusted listener: `unix:<path>` or `tcp:<loopback addr:port>`.
    #[arg(long)]
    listen: Option<ListenAddr>,
    /// Untrusted listener: `unix:<path>` or `tcp:<loopback addr:port>`.
    #[arg(long)]
    untrusted_listen: Option<ListenAddr>,
    /// Origin allowed for `/api` and `/ws` (repeat for several).
    #[arg(long = "allow-origin", value_name = "ORIGIN")]
    allow_origin: Vec<String>,
    /// Local Distill binary (`distill agent --leader stdio`).
    #[arg(long, default_value = "distill")]
    distill_bin: PathBuf,
    /// State directory for keys, subscriptions and sockets. Default: `$HOME/.distill/postudio`.
    #[arg(long)]
    state_dir: Option<PathBuf>,
    /// Directory with the built web app, served with SPA fallback.
    #[arg(long)]
    web_dir: Option<PathBuf>,
    /// Private local JSON file with pinned chat-sync peers and bearer tokens.
    #[arg(long)]
    chat_peer_config: Option<PathBuf>,
    /// Separate public origin for untrusted files and proxied apps.
    #[arg(long)]
    untrusted_origin: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    let Command::Serve(args) = Cli::parse().command;
    let state_dir = match args.state_dir {
        Some(dir) => dir,
        None => {
            let home = std::env::var_os("HOME").context("HOME is not set; pass --state-dir")?;
            PathBuf::from(home).join(".distill/postudio")
        }
    };
    run(NodeConfig {
        owner: args.owner,
        listen: args.listen.unwrap_or_else(listen::default_trusted),
        untrusted_listen: args
            .untrusted_listen
            .unwrap_or_else(listen::default_untrusted),
        allowed_origins: args.allow_origin,
        agent: AgentConfig {
            distill_bin: args.distill_bin,
            ..AgentConfig::default()
        },
        state_dir,
        web_dir: args.web_dir,
        chat_peer_config: args.chat_peer_config,
        untrusted_origin: args.untrusted_origin,
    })
    .await
}
