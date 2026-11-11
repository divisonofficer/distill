//! Dev stack: sandbox, mock model, distill leader and the postudio node. Shared by `dev-stack` and tests/node.rs.

use std::ffi::OsString;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context as _, bail};
use base64::Engine as _;
use distill_node::NodeConfig;
use distill_node::agent::AgentConfig;
use distill_node::listen::ListenAddr;
use distill_test_support::{
    Conversation, LeaderFixture, MockInferenceServer, MockToolCall, TestSandbox, Tool,
};
use serde_json::json;
use tokio::task::JoinHandle;
use tokio::time::Instant;

const TINY_PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

const FALLBACK_REPLY: &str = r#"# 결과

- 첫째 항목
- 둘째 항목

```rust
fn main() {
    println!("hello");
}
```
"#;

/// Running dev stack. Dropping it stops the node and then the leader, mock server and sandbox.
pub struct Stack {
    pub url: String,
    pub cwd: PathBuf,
    task: JoinHandle<anyhow::Result<()>>,
    _leader: LeaderFixture,
    _server: MockInferenceServer,
    _sandbox: TestSandbox,
}

impl Drop for Stack {
    fn drop(&mut self) {
        self.task.abort();
        // Scripts a caller never reached are not a failure for a dev stack.
        let _ = self._server.finish_scripts();
    }
}

/// Mock model script. Conversations are keyed by session id in first-seen order: the first
/// session created on a stack is conversation 1, the second is conversation 2. Later sessions
/// get `FALLBACK_REPLY`. `port` 0 picks free ports.
pub async fn start(port: u16, web_dir: Option<PathBuf>) -> anyhow::Result<Stack> {
    let server = MockInferenceServer::start()
        .await
        .context("start mock server")?;
    let sandbox = TestSandbox::builder().mock_url(server.url()).build();
    let booted = boot(&server, &sandbox, port, web_dir).await;
    match booted {
        Ok((leader, task, cwd, port)) => {
            let url = format!("http://127.0.0.1:{port}");
            let mut stack = Stack {
                url: url.clone(),
                cwd,
                task,
                _leader: leader,
                _server: server,
                _sandbox: sandbox,
            };
            wait_healthy(&mut stack.task, &url).await?;
            Ok(stack)
        }
        Err(e) => {
            // Unserved scripts would make the mock's drop check panic and hide `e`.
            let _ = server.finish_scripts();
            Err(e)
        }
    }
}

async fn boot(
    server: &MockInferenceServer,
    sandbox: &TestSandbox,
    port: u16,
    web_dir: Option<PathBuf>,
) -> anyhow::Result<(LeaderFixture, JoinHandle<anyhow::Result<()>>, PathBuf, u16)> {
    let root = sandbox.root().to_path_buf();
    let cwd = root.join("proj");
    std::fs::create_dir_all(&cwd).context("create proj")?;
    let readme = cwd.join("README.md");
    std::fs::write(&readme, "hello").context("write README.md")?;
    let png = cwd.join("tiny.png");
    let png_bytes = base64::engine::general_purpose::STANDARD
        .decode(TINY_PNG)
        .context("decode tiny.png")?;
    std::fs::write(&png, png_bytes).context("write tiny.png")?;

    server.set_conversations([
        Conversation::nth(1)
            .calls([MockToolCall::new(
                Tool::Edit,
                json!({
                    "file_path": readme,
                    "old_string": "hello",
                    "new_string": "hello from postudio",
                }),
            )])
            .reply("README 수정 완료."),
        Conversation::nth(2)
            .calls([MockToolCall::new(Tool::Read, json!({"target_file": png}))])
            .reply("이미지를 읽었습니다."),
    ]);
    server.set_response(FALLBACK_REPLY);

    let leader = LeaderFixture::start(server, sandbox.workspace(), sandbox)
        .await
        .context("start leader")?;

    let mut extra_env: Vec<(OsString, OsString)> = sandbox.env();
    extra_env.push((
        "GROK_LEADER_SOCKET".into(),
        sandbox.distill_home().join("leader.sock").into_os_string(),
    ));
    let distill_bin = std::env::var_os("GROK_BINARY")
        .map(PathBuf::from)
        .context("GROK_BINARY is required")?;
    let agent = AgentConfig {
        distill_bin,
        extra_env,
        ..AgentConfig::default()
    };

    // Port 0: pick both ports now, after the mock server and leader took theirs.
    let (port, web_port) = if port == 0 {
        let a = std::net::TcpListener::bind("127.0.0.1:0").context("pick port")?;
        let b = std::net::TcpListener::bind("127.0.0.1:0").context("pick port")?;
        (a.local_addr()?.port(), b.local_addr()?.port())
    } else {
        (port, port.checked_add(1).context("port + 1 overflows")?)
    };
    let cfg = NodeConfig {
        owner: "me@example.com".into(),
        listen: ListenAddr::Tcp(SocketAddr::from(([127, 0, 0, 1], port))),
        untrusted_listen: ListenAddr::Tcp(SocketAddr::from(([127, 0, 0, 1], web_port))),
        allowed_origins: vec![
            "http://127.0.0.1:5173".into(),
            format!("http://127.0.0.1:{port}"),
        ],
        agent,
        state_dir: root.join("state"),
        web_dir,
    };
    let task = tokio::spawn(distill_node::run(cfg));
    Ok((leader, task, cwd, port))
}

#[allow(
    clippy::disallowed_methods,
    reason = "localhost-only health probe against the node on 127.0.0.1"
)]
async fn wait_healthy(task: &mut JoinHandle<anyhow::Result<()>>, url: &str) -> anyhow::Result<()> {
    let client = reqwest::Client::new();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if task.is_finished() {
            let outcome = task.await;
            bail!("node exited before /healthz answered: {outcome:?}");
        }
        let ok = client
            .get(format!("{url}/healthz"))
            .timeout(Duration::from_secs(2))
            .send()
            .await
            .is_ok_and(|r| r.status().is_success());
        if ok {
            return Ok(());
        }
        if Instant::now() >= deadline {
            task.abort();
            bail!("node /healthz did not answer within 20s at {url}");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
