#![cfg(unix)]

use std::os::unix::fs::PermissionsExt as _;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use distill_node::account_login::{LoginLimits, routes_with_limits};
use distill_node::agent::AgentConfig;
use distill_node::guard::GuardConfig;
use distill_node::keeper::Keeper;
use distill_node::push::Push;
use distill_node::trusted_router;
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::time::sleep;
use tower::ServiceExt as _;

const OWNER: &str = "owner@example.com";
const ORIGIN: &str = "https://node.example.com";

const FAKE_DISTILL: &str = r##"#!/usr/bin/env python3
import json
import os
import sys
import time

args = sys.argv[1:]
with open(os.environ["ARGS_FILE"], "w", encoding="utf-8") as file:
    json.dump(args, file)
mode = os.environ["FAKE_MODE"]
if len(args) >= 2 and args[:2] == ["accounts", "login-web"]:
    with open(os.environ["PID_FILE"], "w", encoding="utf-8") as file:
        file.write(str(os.getpid()))
    def emit(value):
        print(json.dumps(value), flush=True)
    def pending(url="https://auth.openai.com/oauth/authorize?state=test"):
        emit({"status": "pending", "authorizationUrl": url, "callbackMode": "host_loopback"})
    if mode == "pending" or mode == "timeout":
        pending()
        time.sleep(30)
    elif mode == "complete":
        pending()
        time.sleep(0.1)
        emit({"status": "completed"})
    elif mode == "exit_failure":
        pending()
        emit({"status": "completed"})
        sys.exit(9)
    elif mode == "malformed":
        emit({"status": "pending", "authorizationUrl": "https://auth.openai.com/oauth/authorize", "callbackMode": "host_loopback", "accessToken": "secret-token"})
    elif mode == "bad_url":
        pending("https://auth.openai.com.evil.test/login")
    elif mode == "grok":
        pending("https://auth.x.ai/authorize")
    elif mode == "userinfo":
        pending("https://user:secret@auth.openai.com/login")
    elif mode == "empty_userinfo":
        pending("https://@auth.openai.com/login")
    elif mode == "line_limit":
        sys.stdout.write("x" * 8193)
        sys.stdout.flush()
        time.sleep(30)
    elif mode == "total_limit":
        pending("https://auth.openai.com/login?state=" + "x" * 256)
    elif mode == "failed_event":
        emit({"status": "failed", "error": "secret-provider-error"})
    else:
        sys.exit(10)
else:
    for line in sys.stdin:
        request = json.loads(line)
        method = request.get("method")
        if method == "initialize":
            result = {"authMethods": []}
        elif method == "_x.ai/sessions/list":
            result = {"sessions": []}
        else:
            result = {}
        print(json.dumps({"jsonrpc": "2.0", "id": request["id"], "result": result}), flush=True)
"##;

fn fake_config(dir: &TempDir, mode: &str) -> AgentConfig {
    let executable = dir.path().join("fake-distill.py");
    std::fs::write(&executable, FAKE_DISTILL).unwrap();
    let mut permissions = std::fs::metadata(&executable).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&executable, permissions).unwrap();
    AgentConfig {
        distill_bin: executable,
        args: Vec::new(),
        extra_env: vec![
            (
                "ARGS_FILE".into(),
                dir.path().join("args.json").into_os_string(),
            ),
            ("PID_FILE".into(), dir.path().join("pid").into_os_string()),
            ("FAKE_MODE".into(), mode.into()),
        ],
    }
}

fn limits() -> LoginLimits {
    LoginLimits {
        timeout: Duration::from_secs(2),
        ..LoginLimits::default()
    }
}

fn app(cfg: AgentConfig, limits: LoginLimits) -> Router {
    routes_with_limits(cfg, limits)
}

async fn call(
    app: &Router,
    method: &str,
    path: &str,
    body: Option<Value>,
    owner: Option<&str>,
) -> axum::response::Response {
    let mut builder = Request::builder().method(method).uri(path);
    if let Some(owner) = owner {
        builder = builder.header("Tailscale-User-Login", owner);
    }
    let body = match body {
        Some(body) => {
            builder = builder.header(header::CONTENT_TYPE, "application/json");
            Body::from(body.to_string())
        }
        None => Body::empty(),
    };
    app.clone()
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap()
}

async fn json_response(response: axum::response::Response) -> (StatusCode, Value) {
    let status = response.status();
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let body = if body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&body).unwrap_or(Value::Null)
    };
    (status, body)
}

async fn start(app: &Router, id: &str) -> String {
    let (status, body) = json_response(
        call(
            app,
            "POST",
            "/api/accounts/login",
            Some(json!({"id": id})),
            Some(OWNER),
        )
        .await,
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(body["status"], "starting");
    assert_eq!(body["callbackMode"], "host_loopback");
    assert!(body.get("authorizationUrl").is_none());
    body["jobId"].as_str().unwrap().to_owned()
}

async fn wait_for(app: &Router, job_id: &str, target: &str) -> Value {
    for _ in 0..250 {
        let (status, body) = json_response(
            call(
                app,
                "GET",
                &format!("/api/accounts/login/{job_id}"),
                None,
                Some(OWNER),
            )
            .await,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        if body["status"] == target {
            return body;
        }
        if body["status"] == "failed"
            || body["status"] == "cancelled"
            || body["status"] == "expired"
        {
            panic!("job reached {} before {target}: {body}", body["status"]);
        }
        sleep(Duration::from_millis(10)).await;
    }
    panic!("job did not reach {target}");
}

#[tokio::test]
async fn start_poll_complete_and_fixed_argv() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = fake_config(&dir, "complete");
    let app = app(cfg, limits());
    let job_id = start(&app, "work_1").await;
    let pending = wait_for(&app, &job_id, "pending").await;
    assert_eq!(
        pending["authorizationUrl"],
        "https://auth.openai.com/oauth/authorize?state=test"
    );
    assert_eq!(pending["callbackMode"], "host_loopback");

    let completed = wait_for(&app, &job_id, "completed").await;
    assert!(completed.get("authorizationUrl").is_none());
    let args: Value =
        serde_json::from_slice(&std::fs::read(dir.path().join("args.json")).unwrap()).unwrap();
    assert_eq!(args, json!(["accounts", "login-web", "work_1"]));
}

#[tokio::test]
async fn cancel_is_owner_bound_reaps_child_and_blocks_parallel_login() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(fake_config(&dir, "pending"), limits());
    let job_id = start(&app, "work").await;
    let pending = wait_for(&app, &job_id, "pending").await;
    assert_eq!(
        pending["authorizationUrl"],
        "https://auth.openai.com/oauth/authorize?state=test"
    );

    let (status, _) = json_response(
        call(
            &app,
            "POST",
            "/api/accounts/login",
            Some(json!({"id": "other"})),
            Some(OWNER),
        )
        .await,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    let (status, _) = json_response(
        call(
            &app,
            "POST",
            &format!("/api/accounts/login/{job_id}/cancel"),
            Some(json!({})),
            Some("other@example.com"),
        )
        .await,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, cancelled) = json_response(
        call(
            &app,
            "POST",
            &format!("/api/accounts/login/{job_id}/cancel"),
            Some(json!({})),
            Some(OWNER),
        )
        .await,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(cancelled["status"], "cancelled");
    assert!(cancelled.get("authorizationUrl").is_none());

    let pid: u32 = std::fs::read_to_string(dir.path().join("pid"))
        .unwrap()
        .parse()
        .unwrap();
    for _ in 0..100 {
        if !process_alive(pid) {
            break;
        }
        sleep(Duration::from_millis(10)).await;
    }
    assert!(!process_alive(pid), "cancel must kill and reap login child");
}

fn process_alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[tokio::test]
async fn short_timeout_fails_and_kills_child() {
    let dir = tempfile::tempdir().unwrap();
    let mut limits = limits();
    limits.timeout = Duration::from_secs(1);
    let app = app(fake_config(&dir, "timeout"), limits);
    let job_id = start(&app, "work").await;
    let failed = wait_for(&app, &job_id, "failed").await;
    assert!(failed.get("authorizationUrl").is_none());
    let pid: u32 = std::fs::read_to_string(dir.path().join("pid"))
        .unwrap()
        .parse()
        .unwrap();
    for _ in 0..100 {
        if !process_alive(pid) {
            break;
        }
        sleep(Duration::from_millis(10)).await;
    }
    assert!(!process_alive(pid), "timed-out login child must be reaped");
}

#[tokio::test]
async fn terminal_jobs_expire_and_clear_authorization_url() {
    let dir = tempfile::tempdir().unwrap();
    let mut limits = limits();
    limits.ttl = Duration::from_millis(500);
    let app = app(fake_config(&dir, "complete"), limits);
    let job_id = start(&app, "work").await;
    for _ in 0..100 {
        let (_, body) = json_response(
            call(
                &app,
                "GET",
                &format!("/api/accounts/login/{job_id}"),
                None,
                Some(OWNER),
            )
            .await,
        )
        .await;
        if body["status"] == "completed" {
            assert!(body.get("authorizationUrl").is_none());
            sleep(Duration::from_millis(550)).await;
            let (status, expired) = json_response(
                call(
                    &app,
                    "GET",
                    &format!("/api/accounts/login/{job_id}"),
                    None,
                    Some(OWNER),
                )
                .await,
            )
            .await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(expired["status"], "expired");
            assert!(expired.get("authorizationUrl").is_none());
            return;
        }
        sleep(Duration::from_millis(10)).await;
    }
    panic!("login job did not complete");
}

#[tokio::test]
async fn malformed_urls_and_output_limits_fail_without_leaking_secrets() {
    for (mode, output_limit) in [
        ("malformed", 64 * 1024),
        ("bad_url", 64 * 1024),
        ("grok", 64 * 1024),
        ("userinfo", 64 * 1024),
        ("empty_userinfo", 64 * 1024),
        ("failed_event", 64 * 1024),
        ("line_limit", 64 * 1024),
        ("total_limit", 64),
        ("exit_failure", 64 * 1024),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let mut limits = limits();
        limits.max_output_bytes = output_limit;
        let app = app(fake_config(&dir, mode), limits);
        let job_id = start(&app, "work").await;
        let failed = wait_for(&app, &job_id, "failed").await;
        assert!(failed.get("authorizationUrl").is_none());
        assert!(!failed.to_string().contains("secret"));
        assert!(!failed.to_string().contains("provider-error"));
    }
}

#[tokio::test]
async fn validates_ids_and_rejects_unknown_request_fields() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(fake_config(&dir, "complete"), limits());
    for id in ["-bad", "bad;id", &"x".repeat(65)] {
        let (status, _) = json_response(
            call(
                &app,
                "POST",
                "/api/accounts/login",
                Some(json!({"id": id})),
                Some(OWNER),
            )
            .await,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
    let (status, _) = json_response(
        call(
            &app,
            "POST",
            "/api/accounts/login",
            Some(json!({"id": "work", "flags": ["--help"]})),
            Some(OWNER),
        )
        .await,
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(!dir.path().join("args.json").exists());
}

#[tokio::test]
async fn dropping_router_cancels_and_reaps_login_child() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(fake_config(&dir, "pending"), limits());
    let job_id = start(&app, "work").await;
    wait_for(&app, &job_id, "pending").await;
    let pid: u32 = std::fs::read_to_string(dir.path().join("pid"))
        .unwrap()
        .parse()
        .unwrap();

    drop(app);
    for _ in 0..100 {
        if !process_alive(pid) {
            break;
        }
        sleep(Duration::from_millis(10)).await;
    }
    assert!(
        !process_alive(pid),
        "dropping node router must kill and reap login child"
    );
}

#[tokio::test]
async fn actual_trusted_router_guards_login_routes_by_owner_and_origin() {
    let dir = tempfile::tempdir().unwrap();
    let agent = fake_config(&dir, "pending");
    let keeper = Keeper::start(agent.clone()).await.unwrap();
    let state_dir = dir.path().join("state");
    let push = Arc::new(Push::load_or_create(&state_dir).unwrap());
    let chat_store = distill_node::chat_store::Store::open(&state_dir.join("chats")).unwrap();
    let guard = Arc::new(GuardConfig {
        owner: OWNER.to_owned(),
        allowed_origins: vec![ORIGIN.to_owned()],
    });
    let app = trusted_router(keeper, push, agent, guard, None, chat_store);

    let mut request = Request::builder()
        .method("POST")
        .uri("/api/accounts/login")
        .header(header::CONTENT_TYPE, "application/json")
        .header("Tailscale-User-Login", OWNER)
        .header(header::ORIGIN, "https://evil.example")
        .body(Body::from(json!({"id": "work"}).to_string()))
        .unwrap();
    let blocked = app.clone().oneshot(request).await.unwrap();
    assert_eq!(blocked.status(), StatusCode::FORBIDDEN);

    request = Request::builder()
        .method("POST")
        .uri("/api/accounts/login")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ORIGIN, ORIGIN)
        .body(Body::from(json!({"id": "work"}).to_string()))
        .unwrap();
    let blocked = app.clone().oneshot(request).await.unwrap();
    assert_eq!(blocked.status(), StatusCode::FORBIDDEN);

    request = Request::builder()
        .method("POST")
        .uri("/api/accounts/login")
        .header(header::CONTENT_TYPE, "application/json")
        .header("Tailscale-User-Login", OWNER)
        .header(header::ORIGIN, ORIGIN)
        .body(Body::from(json!({"id": "work"}).to_string()))
        .unwrap();
    let started = app.clone().oneshot(request).await.unwrap();
    assert_eq!(started.status(), StatusCode::ACCEPTED);
    let (_, body) = json_response(started).await;
    let job_id = body["jobId"].as_str().unwrap();

    let cancelled = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/accounts/login/{job_id}/cancel"))
                .header(header::CONTENT_TYPE, "application/json")
                .header("Tailscale-User-Login", OWNER)
                .header(header::ORIGIN, ORIGIN)
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cancelled.status(), StatusCode::OK);
}
