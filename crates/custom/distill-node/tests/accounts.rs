#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::Duration;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use distill_node::accounts::routes;
use distill_node::agent::AgentConfig;
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::time::timeout;
use tower::ServiceExt;

const FAKE_DISTILL: &str = r##"#!/usr/bin/env python3
import json
import os
import stat
import sys
import time

args = sys.argv[1:]
log_path = os.environ["ARGS_LOG"]
if os.path.exists(log_path):
    with open(log_path, "r", encoding="utf-8") as log:
        calls = json.load(log)
else:
    calls = []
calls.append(args)
with open(log_path, "w", encoding="utf-8") as log:
    json.dump(calls, log)

mode = os.environ["FAKE_MODE"]
routing = {"enabled": True, "strategy": "ordered_first", "options": [{"id": "primary", "account_id": "work", "main_model": "model-a", "credential_secret": "routing-option-secret"}]}
if args == ["accounts", "routing", "get"]:
    if mode == "unexpected-routing":
        print(json.dumps({**routing, "api_key": "routing-secret"}))
    elif os.path.exists(os.environ["ROUTING_LOG"]):
        with open(os.environ["ROUTING_LOG"], "r", encoding="utf-8") as log:
            print(json.dumps(json.load(log)["data"]))
    else:
        print(json.dumps(routing))
    sys.exit(0)
if args[:4] == ["accounts", "routing", "set", "--file"]:
    filename = args[4]
    info = os.stat(filename)
    with open(filename, "r", encoding="utf-8") as source:
        saved = json.load(source)
    with open(os.environ["ROUTING_LOG"], "w", encoding="utf-8") as log:
        json.dump({"path": filename, "mode": stat.S_IMODE(info.st_mode), "data": saved}, log)
    if mode == "routing-error":
        print("routing-secret-error", file=sys.stderr)
        sys.exit(9)
    if mode == "routing-timeout":
        time.sleep(60)
    if mode == "routing-invalid-output":
        print("not json")
        sys.exit(0)
    print(json.dumps({"ok": True}))
    sys.exit(0)


if mode == "timeout":
    time.sleep(60)
if mode == "error":
    print("provider-secret-error", file=sys.stderr)
    sys.exit(9)
if mode == "large":
    sys.stdout.write("x" * (2 * 1024 * 1024))
    sys.stdout.flush()
    time.sleep(60)

quota = {
    "account_id": "work",
    "checked_at": "2026-11-11T00:00:00Z",
    "fetched_at": "2026-11-11T00:00:00Z",
    "status": "available",
    "remaining_percent": 72.5,
    "detail": {"api_secret": "quota-secret"},
}
if args == ["accounts", "list"]:
    print(json.dumps([{
        "id": "work",
        "provider": "claude",
        "label": "Work",
        "legacy": False,
        "connected": True,
        "email": "owner@example.com",
        "connection_error": None,
        "quota": quota,
        "credential_secret": "account-secret",
    }]))
elif len(args) == 3 and args[:2] == ["accounts", "usage"]:
    print(json.dumps({**quota, "credential_secret": "usage-secret"}))
else:
    print(json.dumps({"ok": True, "credential_secret": "mutation-secret"}))
"##;

fn fake_config(dir: &TempDir, mode: &str) -> AgentConfig {
    let executable = dir.path().join("fake-distill.py");
    std::fs::write(&executable, FAKE_DISTILL).unwrap();
    let mut permissions = std::fs::metadata(&executable).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&executable, permissions).unwrap();
    AgentConfig {
        distill_bin: executable,
        args: ["must", "be", "ignored"].map(Into::into).to_vec(),
        extra_env: vec![
            (
                "ARGS_LOG".into(),
                dir.path().join("args.json").into_os_string(),
            ),
            (
                "ROUTING_LOG".into(),
                dir.path().join("routing.json").into_os_string(),
            ),
            ("FAKE_MODE".into(), mode.into()),
        ],
    }
}

async fn call(
    cfg: AgentConfig,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> axum::response::Response {
    let mut builder = Request::builder().method(method).uri(path);
    let body = if let Some(body) = body {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
        Body::from(body.to_string())
    } else {
        Body::empty()
    };
    routes(cfg)
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap()
}

async fn response_json(response: axum::response::Response) -> (StatusCode, Value) {
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
async fn list_and_usage_forward_only_safe_documented_fields() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = fake_config(&dir, "ok");

    let (status, listed) =
        response_json(call(cfg.clone(), "GET", "/api/accounts", None).await).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(listed[0]["id"], "work");
    assert_eq!(listed[0]["email"], "owner@example.com");
    assert_eq!(listed[0]["quota"]["remaining_percent"], 72.5);
    assert!(listed[0].get("credential_secret").is_none());
    assert!(listed[0]["quota"].get("detail").is_none());
    assert!(!listed.to_string().contains("account-secret"));
    assert!(!listed.to_string().contains("quota-secret"));

    let (status, usage) = response_json(
        call(
            cfg,
            "POST",
            "/api/accounts/usage",
            Some(json!({"id":"work"})),
        )
        .await,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(usage["status"], "available");
    assert!(usage.get("detail").is_none());
    assert!(usage.get("credential_secret").is_none());
    assert!(!usage.to_string().contains("usage-secret"));
}

#[tokio::test]
async fn command_uses_fixed_args_without_shell_or_agent_args() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = fake_config(&dir, "ok");
    let marker = dir.path().join("shell-ran");
    let label = format!("$(touch {})", marker.display());
    assert!(label.chars().count() <= 120);

    let (status, body) = response_json(
        call(
            cfg,
            "POST",
            "/api/accounts/add",
            Some(json!({"id":"fresh","provider":"claude","label":label})),
        )
        .await,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({"ok": true}));
    let args: Value =
        serde_json::from_slice(&std::fs::read(dir.path().join("args.json")).unwrap()).unwrap();
    assert_eq!(
        args[0],
        json!([
            "accounts",
            "add",
            "fresh",
            "--provider",
            "claude",
            "--label",
            label
        ])
    );
    assert!(!Path::new(&marker).exists());
}

#[tokio::test]
async fn rejects_invalid_ids_labels_and_providers_before_spawning() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = fake_config(&dir, "ok");
    for id in ["-bad", "has;separator", &"x".repeat(65)] {
        let (status, _) = response_json(
            call(
                cfg.clone(),
                "POST",
                "/api/accounts/add",
                Some(json!({"id":id,"provider":"claude","label":"Work"})),
            )
            .await,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
    for (provider, label) in [
        ("unknown", "Work"),
        ("claude", "  "),
        ("claude", &"x".repeat(121)),
    ] {
        let (status, _) = response_json(
            call(
                cfg.clone(),
                "POST",
                "/api/accounts/add",
                Some(json!({"id":"valid","provider":provider,"label":label})),
            )
            .await,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
    assert!(!dir.path().join("args.json").exists());
}

#[tokio::test]
async fn routing_get_sanitizes_unexpected_cli_fields() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = fake_config(&dir, "unexpected-routing");
    let (status, body) = response_json(call(cfg, "GET", "/api/accounts/routing", None).await).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        json!({
            "enabled": true,
            "strategy": "ordered_first",
            "options": [{
                "id": "primary",
                "account_id": "work",
                "main_model": "model-a",
                "worker_model": null
            }]
        })
    );
    assert!(!body.to_string().contains("routing-secret"));
    assert!(!body.to_string().contains("routing-option-secret"));
    let args: Value =
        serde_json::from_slice(&std::fs::read(dir.path().join("args.json")).unwrap()).unwrap();
    assert_eq!(args, json!([["accounts", "routing", "get"]]));
}

#[tokio::test]
async fn routing_set_uses_private_temp_file_and_returns_readback() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = fake_config(&dir, "ok");
    let desired = json!({
        "enabled": true,
        "strategy": "highest_remaining",
        "options": [{
            "id": "backup",
            "account_id": "work",
            "main_model": "main-model",
            "worker_model": "worker-model"
        }]
    });
    let (status, response) =
        response_json(call(cfg, "POST", "/api/accounts/routing", Some(desired.clone())).await)
            .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(response, desired);
    let args: Value =
        serde_json::from_slice(&std::fs::read(dir.path().join("args.json")).unwrap()).unwrap();
    let cli_path = Path::new(args[0][4].as_str().unwrap());
    assert_eq!(args[0][0], "accounts");
    assert_eq!(args[0][1], "routing");
    assert_eq!(args[0][2], "set");
    assert_eq!(args[0][3], "--file");
    assert_eq!(args[1], json!(["accounts", "routing", "get"]));
    assert!(cli_path.is_absolute());
    assert!(cli_path.starts_with(std::env::temp_dir()));
    assert!(
        cli_path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with(".distill-routing-")
    );
    assert!(!cli_path.starts_with(dir.path()));
    assert_eq!(
        std::fs::metadata(cli_path).unwrap_err().kind(),
        std::io::ErrorKind::NotFound
    );
    let saved: Value =
        serde_json::from_slice(&std::fs::read(dir.path().join("routing.json")).unwrap()).unwrap();
    assert_eq!(saved["mode"], 0o600);
    assert_eq!(saved["data"], desired);
    assert_eq!(saved["path"], args[0][4]);
}

#[tokio::test]
async fn routing_set_rejects_invalid_input_before_spawning() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = fake_config(&dir, "ok");
    let mut invalid = vec![
        json!({"enabled": true, "strategy": "unknown", "options": []}),
        json!({"enabled": true, "strategy": "ordered_first", "options": [], "secret": "x"}),
        json!({"enabled": true, "strategy": "ordered_first", "options": [{"id":"one", "account_id":"work", "main_model":"main", "extra":"x"}]}),
        json!({"enabled": true, "strategy": "ordered_first", "options": [{"id":"One", "account_id":"work", "main_model":"main"}]}),
        json!({"enabled": true, "strategy": "ordered_first", "options": [{"id":"a".repeat(65), "account_id":"work", "main_model":"main"}]}),
        json!({"enabled": true, "strategy": "ordered_first", "options": [{"id":"one", "account_id":"Bad", "main_model":"main"}]}),
        json!({"enabled": true, "strategy": "ordered_first", "options": [{"id":"one", "account_id":"w".repeat(65), "main_model":"main"}]}),
        json!({"enabled": true, "strategy": "ordered_first", "options": [{"id":"one", "account_id":"work", "main_model":"  "}]}),
        json!({"enabled": true, "strategy": "ordered_first", "options": [{"id":"one", "account_id":"work", "main_model":"m", "worker_model":""}]}),
        json!({"enabled": true, "strategy": "ordered_first", "options": [{"id":"one", "account_id":"work", "main_model":"x".repeat(257)}]}),
        json!({"enabled": true, "strategy": "ordered_first", "options": [{"id":"one", "account_id":"work", "main_model":"main", "worker_model":"x".repeat(257)}]}),
        json!({"enabled": true, "strategy": "ordered_first", "options": [
            {"id":"one", "account_id":"work", "main_model":"main"},
            {"id":"one", "account_id":"work", "main_model":"main"}
        ]}),
    ];
    invalid.push(json!({
        "enabled": false,
        "strategy": "ordered_first",
        "options": (0..65).map(|i| json!({"id": format!("id{i}"), "account_id":"work", "main_model":"main"})).collect::<Vec<_>>()
    }));
    for input in invalid {
        let response = call(cfg.clone(), "POST", "/api/accounts/routing", Some(input)).await;
        assert!(matches!(
            response.status(),
            StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY
        ));
    }
    assert!(!dir.path().join("args.json").exists());
}

#[tokio::test]
async fn routing_set_errors_do_not_leak_cli_details_and_remove_temp_file() {
    for mode in ["routing-error", "routing-invalid-output", "routing-timeout"] {
        let dir = tempfile::tempdir().unwrap();
        let cfg = fake_config(&dir, mode);
        let request = json!({
            "enabled": true,
            "strategy": "ordered_first",
            "options": [{ "id": "one", "account_id": "work", "main_model": "main" }]
        });
        let pending = call(cfg, "POST", "/api/accounts/routing", Some(request));
        let response = if mode == "routing-timeout" {
            timeout(Duration::from_secs(17), pending)
                .await
                .expect("routing command must time out")
        } else {
            pending.await
        };
        let (status, body) = response_json(response).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert_eq!(body, json!({"error":"Account command failed."}));
        assert!(!body.to_string().contains("routing-secret-error"));
        let saved: Value =
            serde_json::from_slice(&std::fs::read(dir.path().join("routing.json")).unwrap())
                .unwrap();
        assert_eq!(
            std::fs::metadata(saved["path"].as_str().unwrap())
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::NotFound
        );
    }
}

#[tokio::test]
async fn cancelled_routing_set_removes_temp_file() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = fake_config(&dir, "routing-timeout");
    let request = json!({
        "enabled": true,
        "strategy": "ordered_first",
        "options": [{ "id": "one", "account_id": "work", "main_model": "main" }]
    });
    let task = tokio::spawn(call(cfg, "POST", "/api/accounts/routing", Some(request)));
    let log_path = dir.path().join("routing.json");
    tokio::time::timeout(Duration::from_secs(5), async {
        while !log_path.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("CLI must record temp path before cancellation");
    let saved: Value = serde_json::from_slice(&std::fs::read(log_path).unwrap()).unwrap();
    let temp_path = Path::new(saved["path"].as_str().unwrap()).to_path_buf();
    task.abort();
    let _ = task.await;
    assert_eq!(
        std::fs::metadata(temp_path).unwrap_err().kind(),
        std::io::ErrorKind::NotFound
    );
}

#[tokio::test]
async fn cli_failures_and_output_limit_return_generic_errors() {
    for mode in ["error", "large"] {
        let dir = tempfile::tempdir().unwrap();
        let cfg = fake_config(&dir, mode);
        let (status, body) = response_json(call(cfg, "GET", "/api/accounts", None).await).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert_eq!(body, json!({"error": "Account command failed."}));
        assert!(!body.to_string().contains("provider-secret-error"));
    }
}

#[tokio::test]
async fn timed_out_command_returns_generic_error_and_is_killed() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = fake_config(&dir, "timeout");
    let response = timeout(
        Duration::from_secs(17),
        call(cfg, "GET", "/api/accounts", None),
    )
    .await
    .expect("account command must time out after its 15 second limit");
    let (status, body) = response_json(response).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(body, json!({"error": "Account command failed."}));
}
