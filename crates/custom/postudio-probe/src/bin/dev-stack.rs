//! Local dev stack: fake model, sandbox, leader and postudio node. Waits for Ctrl-C.

use std::path::PathBuf;

use clap::Parser;
use postudio_probe::stack;

#[derive(Debug, Parser)]
#[command(name = "dev-stack")]
struct Cli {
    /// Trusted listener port on 127.0.0.1; the untrusted listener uses port + 1.
    #[arg(long, default_value_t = 7410)]
    port: u16,
    /// Built web app to serve (e.g. apps/postudio/build).
    #[arg(long, env = "POSTUDIO_WEB_DIR")]
    web_dir: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let stack = stack::start(cli.port, cli.web_dir).await?;
    println!("url: {}", stack.url);
    println!("cwd: {}", stack.cwd.display());
    println!(
        "curl -H 'Tailscale-User-Login: me@example.com' {}/api/sessions",
        stack.url
    );
    tokio::signal::ctrl_c().await?;
    drop(stack);
    Ok(())
}
