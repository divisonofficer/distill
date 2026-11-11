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
import sys
import time

args = sys.argv[1:]
with open(os.environ["ARGS_LOG"], "w", encoding="utf-8") as log:
    json.dump(args, log)

mode = os.environ["FAKE_MODE"]
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
        args,
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
