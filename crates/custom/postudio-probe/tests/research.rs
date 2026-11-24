//! Real leader and native research API with deterministic, local-only inference.

use std::{path::PathBuf, process::Command, time::Duration};

use distill_test_support::{Conversation, MockToolCall, Tool};
use futures_util::{SinkExt as _, StreamExt as _};
use postudio_probe::stack::{self, Stack};
use serde_json::{Value, json};
use tokio::time::{Instant, timeout};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, client::IntoClientRequest, http::HeaderValue},
};

const OWNER: &str = "me@example.com";
const ORIGIN: &str = "http://127.0.0.1:5173";

fn request(stack: &Stack, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
    reqwest::Client::new()
        .request(method, format!("{}{path}", stack.url))
        .header("Tailscale-User-Login", OWNER)
        .header("Origin", ORIGIN)
}

async fn body(response: reqwest::Response) -> Value {
    let status = response.status();
    let text = response.text().await.expect("response text");
    assert!(status.is_success(), "{status}: {text}");
    serde_json::from_str(&text).expect("JSON response")
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires GROK_BINARY, ORX_BINARY and POSTUDIO_RESEARCH_MCP_BIN"]
async fn owner_research_chat_uses_local_mock_stack() {
    let orx = PathBuf::from(std::env::var_os("ORX_BINARY").expect("ORX_BINARY"));
    assert!(orx.is_file());
    assert!(std::env::var_os("POSTUDIO_RESEARCH_MCP_BIN").is_some());
    let stack = stack::start_with_openresearch_bin(0, None, Some(orx))
        .await
        .expect("local mock stack");
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.name", "Fixture"],
        vec!["config", "user.email", "fixture@invalid"],
        vec!["add", "."],
        vec!["commit", "-qm", "fixture"],
    ] {
        assert!(
            Command::new("git")
                .args(args)
                .current_dir(&stack.cwd)
                .status()
                .unwrap()
                .success()
        );
    }
    assert_eq!(
        body(
            request(&stack, reqwest::Method::POST, "/api/research/connect")
                .json(&json!({}))
                .send()
                .await
                .unwrap()
        )
        .await["connected"],
        true
    );
    let project = body(
        request(&stack, reqwest::Method::POST, "/api/research/projects")
            .json(&json!({"name":"real core fixture","path":stack.cwd}))
            .send()
            .await
            .unwrap(),
    )
    .await;
    let project = project.get("project").unwrap_or(&project)["id"]
        .as_str()
        .expect("project id")
        .to_owned();
    let prefix = format!("/api/research/projects/{project}");
    let experiments = body(request(&stack, reqwest::Method::POST, &format!("{prefix}/experiments")).json(&json!({"title":"native baseline","description":"fixture","runCommand":"python3 -c 'print(42)'"})).send().await.unwrap()).await;
    let experiment = experiments["experiments"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["title"] == "native baseline")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let path = format!("{prefix}/experiments/{experiment}");
    let workspace = body(
        request(&stack, reqwest::Method::POST, &format!("{path}/workspace"))
            .json(&json!({}))
            .send()
            .await
            .unwrap(),
    )
    .await;
    let snapshot = body(
        request(&stack, reqwest::Method::GET, &format!("{path}/snapshot"))
            .send()
            .await
            .unwrap(),
    )
    .await;
    let chat = body(
        request(&stack, reqwest::Method::POST, &format!("{path}/chat"))
            .json(&json!({"requestId":"fixture-1"}))
            .send()
            .await
            .unwrap(),
    )
    .await;
    let sid = chat["sessionId"]
        .as_str()
        .expect("actual core session id")
        .to_owned();
    let repeated = body(
        request(&stack, reqwest::Method::POST, &format!("{path}/chat"))
            .json(&json!({"requestId":"fixture-1"}))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(repeated["sessionId"], sid);
    let sessions = body(
        request(&stack, reqwest::Method::GET, "/api/sessions")
            .send()
            .await
            .unwrap(),
    )
    .await;
    let row = sessions["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["sessionId"] == sid)
        .expect("actual leader lists session");
    assert_eq!(
        row["cwd"], workspace["cwd"],
        "core must reuse native workspace"
    );
    let tool = Tool::Mcp {
        server: "postudio-research".into(),
        tool: "request_run".into(),
    };
    stack.set_conversations([Conversation::nth(1)
        .stall(Duration::from_secs(2))
        .calls([
            MockToolCall::new(
                Tool::SearchTool,
                json!({"query":"postudio-research request_run","limit":5}),
            ),
            MockToolCall::new(
                tool.clone(),
                json!({"expectedCommit":snapshot["headCommit"],"targetId":"local"}),
            )
            .result_contains("postudioResearch"),
        ])
        .reply("Approval request staged; execution has not started.")]);
    let mut ws_request = format!("{}/ws/session/{sid}", stack.url.replacen("http", "ws", 1))
        .into_client_request()
        .unwrap();
    ws_request
        .headers_mut()
        .insert("Tailscale-User-Login", HeaderValue::from_static(OWNER));
    ws_request
        .headers_mut()
        .insert("Origin", HeaderValue::from_static(ORIGIN));
    let (mut socket, _) = connect_async(ws_request).await.expect("session socket");
    let mut log = Vec::new();
    let mut prompted = false;
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        let frame = timeout(
            deadline.saturating_duration_since(Instant::now()),
            socket.next(),
        )
        .await
        .expect("turn timeout")
        .expect("socket closed")
        .unwrap();
        let Message::Text(text) = frame else {
            continue;
        };
        let value: Value = serde_json::from_str(&text).unwrap();
        if value["t"] == "replay.end" && !prompted {
            prompted = true;
            let ready_deadline = Instant::now() + Duration::from_secs(10);
            loop {
                let text = std::fs::read_to_string(stack.distill_home().join("leader.log"))
                    .unwrap_or_default();
                if text.contains("Registered MCP tool 'postudio-research__request_run'") {
                    break;
                }
                assert!(
                    Instant::now() < ready_deadline,
                    "MCP registry did not initialize"
                );
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            socket.send(Message::Text(json!({"t":"prompt","text":"Stage a local execution approval for this experiment. Do not approve or execute it."}).to_string().into())).await.unwrap();
        }
        if value["t"] == "permission.request" {
            socket
                .send(Message::Text(
                    json!({"t":"permission","id":value["id"],"option_id":"allow-once"})
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();
        }
        let done = value["t"] == "turn.end";
        log.push(value);
        if done {
            break;
        }
    }
    if !serde_json::to_string(&log)
        .unwrap()
        .contains("postudioResearch")
    {
        if let Ok(text) = std::fs::read_to_string(stack.distill_home().join("leader.log")) {
            for line in text
                .lines()
                .filter(|line| {
                    (line.contains("mcp") || line.contains("MCP"))
                        && !line.contains("POSTUDIO_RESEARCH_TOKEN")
                })
                .take(30)
            {
                eprintln!("MCP diagnostic: {line}");
            }
        }
    }
    assert!(
        serde_json::to_string(&log)
            .unwrap()
            .contains("postudioResearch"),
        "missing tool result card: {log:?}"
    );
    let conversations = stack.conversations();
    assert!(
        conversations
            .iter()
            .any(|conversation| conversation.saw_tool_call(tool.clone())),
        "MCP tool not called"
    );
    let recorded = format!("{conversations:?}");
    assert!(
        recorded.contains("Research reference data"),
        "missing authoritative context"
    );
    assert!(recorded.contains(&experiment));
    assert!(
        stack.script_violations().is_empty(),
        "{:?}",
        stack.script_violations()
    );
    let refs = body(
        request(&stack, reqwest::Method::GET, &format!("{prefix}/refs"))
            .send()
            .await
            .unwrap(),
    )
    .await;
    let approval = refs["approvals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|approval| approval["sessionId"] == sid)
        .expect("model staged actual approval");
    let runs = body(
        request(&stack, reqwest::Method::GET, &format!("{prefix}/runs"))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert!(
        runs["runs"].as_array().unwrap().is_empty(),
        "model must not execute before human approval"
    );
    let approved = body(
        request(
            &stack,
            reqwest::Method::POST,
            &format!("{prefix}/approvals/{}", approval["id"].as_str().unwrap()),
        )
        .json(&json!({"approved":true}))
        .send()
        .await
        .unwrap(),
    )
    .await;
    let run = approved["run"]["id"]
        .as_str()
        .expect("actual submitted run");
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let state = body(
            request(
                &stack,
                reqwest::Method::GET,
                &format!("{prefix}/runs/{run}"),
            )
            .send()
            .await
            .unwrap(),
        )
        .await;
        if state["status"] == "done" {
            assert_eq!(state["exitCode"], 0);
            break;
        }
        assert_ne!(state["status"], "failed", "{state}");
        assert!(Instant::now() < deadline, "native run timeout: {state}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    socket.close(None).await.unwrap();
}
