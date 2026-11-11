use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::{Body, Bytes, to_bytes};
use axum::extract::State;
use axum::http::{HeaderMap, Request, StatusCode, header};
use axum::routing::post;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use distill_node::push::Push;
use ring::signature::{ECDSA_P256_SHA256_FIXED, UnparsedPublicKey};
use serde_json::{Value, json};
use tower::ServiceExt;

// RFC 8291 Section 5 receiver key and auth secret.
const UA_PUBLIC: &str =
    "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4";
const AUTH_SECRET: &str = "BTBZMqHH6r4Tts7J_aSIgg";

fn subscription(endpoint: &str) -> Value {
    json!({
        "endpoint": endpoint,
        "expirationTime": null,
        "keys": { "p256dh": UA_PUBLIC, "auth": AUTH_SECRET },
    })
}

fn read_store(dir: &Path) -> Vec<Value> {
    let bytes = std::fs::read(dir.join("push-subscriptions.json")).unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn endpoints(dir: &Path) -> Vec<String> {
    read_store(dir)
        .iter()
        .map(|s| s["endpoint"].as_str().unwrap().to_owned())
        .collect()
}

async fn post_json(app: Router, path: &str, body: &Value) -> StatusCode {
    let req = Request::builder()
        .method("POST")
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    app.oneshot(req).await.unwrap().status()
}

#[test]
fn load_or_create_reloads_same_key() {
    let dir = tempfile::tempdir().unwrap();
    let first = Push::load_or_create(dir.path())
        .unwrap()
        .public_key_b64url();
    let second = Push::load_or_create(dir.path())
        .unwrap()
        .public_key_b64url();
    assert_eq!(first, second);
    let key = URL_SAFE_NO_PAD.decode(&first).unwrap();
    assert_eq!(key.len(), 65);
    assert_eq!(key[0], 0x04);
}

#[cfg(unix)]
#[test]
fn vapid_file_is_private() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("postudio");
    Push::load_or_create(&state).unwrap();
    let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&state.join("vapid.json")), 0o600);
    assert_eq!(mode(&state), 0o700);
}

#[tokio::test]
async fn key_route_returns_public_key() {
    let dir = tempfile::tempdir().unwrap();
    let push = Arc::new(Push::load_or_create(dir.path()).unwrap());
    let expected = push.public_key_b64url();
    let req = Request::builder()
        .uri("/api/push/key")
        .body(Body::empty())
        .unwrap();
    let res = push.routes().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body: Value =
        serde_json::from_slice(&to_bytes(res.into_body(), 4096).await.unwrap()).unwrap();
    assert_eq!(body["publicKey"], expected);
}

#[tokio::test]
async fn subscribe_then_unsubscribe_persists() {
    let dir = tempfile::tempdir().unwrap();
    let push = Arc::new(Push::load_or_create(dir.path()).unwrap());
    let a = "https://push.example.net/a";
    let b = "https://push.example.net/b";

    assert_eq!(
        post_json(
            push.clone().routes(),
            "/api/push/subscribe",
            &subscription(a)
        )
        .await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        post_json(
            push.clone().routes(),
            "/api/push/subscribe",
            &subscription(a)
        )
        .await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        post_json(
            push.clone().routes(),
            "/api/push/subscribe",
            &subscription(b)
        )
        .await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(endpoints(dir.path()), [a, b]);
    assert_eq!(read_store(dir.path())[0]["keys"]["auth"], AUTH_SECRET);

    let un = json!({ "endpoint": a });
    assert_eq!(
        post_json(push.clone().routes(), "/api/push/unsubscribe", &un).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(endpoints(dir.path()), [b]);

    // Reload from disk: the store survives a restart.
    drop(push);
    let reloaded = Push::load_or_create(dir.path()).unwrap();
    assert_eq!(
        post_json(
            Arc::new(reloaded).routes(),
            "/api/push/unsubscribe",
            &json!({ "endpoint": b })
        )
        .await,
        StatusCode::NO_CONTENT
    );
    assert!(endpoints(dir.path()).is_empty());
}

#[tokio::test]
async fn invalid_subscriptions_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let push = Arc::new(Push::load_or_create(dir.path()).unwrap());
    let ok = "https://push.example.net/ok";

    let mut bad_endpoint = subscription("http://push.example.net/x");
    assert_eq!(
        post_json(push.clone().routes(), "/api/push/subscribe", &bad_endpoint).await,
        StatusCode::BAD_REQUEST
    );
    bad_endpoint["endpoint"] = json!("not a url");
    assert_eq!(
        post_json(push.clone().routes(), "/api/push/subscribe", &bad_endpoint).await,
        StatusCode::BAD_REQUEST
    );

    let mut short_key = subscription(ok);
    short_key["keys"]["p256dh"] = json!(URL_SAFE_NO_PAD.encode([0x04u8; 64]));
    assert_eq!(
        post_json(push.clone().routes(), "/api/push/subscribe", &short_key).await,
        StatusCode::BAD_REQUEST
    );

    let mut wrong_prefix = subscription(ok);
    let mut point = URL_SAFE_NO_PAD.decode(UA_PUBLIC).unwrap();
    point[0] = 0x02;
    wrong_prefix["keys"]["p256dh"] = json!(URL_SAFE_NO_PAD.encode(point));
    assert_eq!(
        post_json(push.clone().routes(), "/api/push/subscribe", &wrong_prefix).await,
        StatusCode::BAD_REQUEST
    );

    let mut bad_auth = subscription(ok);
    bad_auth["keys"]["auth"] = json!(URL_SAFE_NO_PAD.encode([1u8; 15]));
    assert_eq!(
        post_json(push.clone().routes(), "/api/push/subscribe", &bad_auth).await,
        StatusCode::BAD_REQUEST
    );

    let not_json = Request::builder()
        .method("POST")
        .uri("/api/push/subscribe")
        .body(Body::from("{"))
        .unwrap();
    assert_eq!(
        push.clone()
            .routes()
            .oneshot(not_json)
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );

    assert!(!dir.path().join("push-subscriptions.json").exists());
}

#[derive(Clone, Default)]
struct Seen(Arc<Mutex<Vec<(HeaderMap, Bytes)>>>);

async fn accept(State(seen): State<Seen>, headers: HeaderMap, body: Bytes) -> StatusCode {
    seen.0.lock().unwrap().push((headers, body));
    StatusCode::CREATED
}

async fn gone() -> StatusCode {
    StatusCode::GONE
}

/// Fake push service on 127.0.0.1. `/ok` records requests, `/gone` answers 410.
async fn fake_push_service() -> (SocketAddr, Seen) {
    let seen = Seen::default();
    let app = Router::new()
        .route("/ok", post(accept))
        .route("/gone", post(gone))
        .with_state(seen.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await });
    (addr, seen)
}

fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> &'a str {
    headers.get(name).unwrap().to_str().unwrap()
}

#[tokio::test]
async fn notify_all_sends_encrypted_vapid_request_and_drops_gone() {
    let (addr, seen) = fake_push_service().await;
    let origin = format!("http://{addr}");
    let dir = tempfile::tempdir().unwrap();
    let subs = json!([
        subscription(&format!("{origin}/ok")),
        subscription(&format!("{origin}/gone"))
    ]);
    std::fs::write(dir.path().join("push-subscriptions.json"), subs.to_string()).unwrap();

    let push = Push::load_or_create(dir.path()).unwrap();
    let public_key = push.public_key_b64url();
    let payload = json!({ "title": "t", "body": "b", "url": "/s/x", "tag": "x" });
    push.notify_all(&payload).await;

    // 410 subscription removed and store rewritten; the live one stays.
    assert_eq!(endpoints(dir.path()), [format!("{origin}/ok")]);

    let got = seen.0.lock().unwrap();
    assert_eq!(got.len(), 1);
    let (headers, body) = &got[0];
    assert_eq!(header_str(headers, "content-encoding"), "aes128gcm");
    assert_eq!(header_str(headers, "ttl"), "86400");
    assert_eq!(header_str(headers, "urgency"), "high");
    assert_eq!(
        header_str(headers, "content-type"),
        "application/octet-stream"
    );

    // VAPID: `vapid t=<jwt>, k=<key>`; JWT is ES256 over header.claims.
    let auth = header_str(headers, "authorization");
    let rest = auth.strip_prefix("vapid t=").unwrap();
    let (jwt, k) = rest.split_once(", k=").unwrap();
    assert_eq!(k, public_key);
    let parts: Vec<&str> = jwt.split('.').collect();
    assert_eq!(parts.len(), 3);
    let header: Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[0]).unwrap()).unwrap();
    assert_eq!(header, json!({ "typ": "JWT", "alg": "ES256" }));
    let claims: Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[1]).unwrap()).unwrap();
    assert_eq!(claims["aud"], origin);
    assert_eq!(claims["sub"], "mailto:me@example.com");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let exp = claims["exp"].as_u64().unwrap();
    assert!(
        exp > now + 11 * 3600 && exp <= now + 12 * 3600 + 5,
        "exp {exp} vs now {now}"
    );
    let key = URL_SAFE_NO_PAD.decode(k).unwrap();
    let sig = URL_SAFE_NO_PAD.decode(parts[2]).unwrap();
    UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, key)
        .verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &sig)
        .expect("VAPID JWT signature verifies with the advertised key");

    // aes128gcm header: salt(16) | rs(4096, BE) | idlen(65) | keyid(65-byte sender point).
    let plaintext_len = payload.to_string().len();
    assert_eq!(body.len(), 86 + plaintext_len + 1 + 16);
    assert_eq!(&body[16..20], &4096u32.to_be_bytes());
    assert_eq!(body[20], 65);
    assert_eq!(body[21], 0x04);
}

#[tokio::test]
async fn notify_all_with_no_subscriptions_is_noop() {
    let dir = tempfile::tempdir().unwrap();
    let push = Push::load_or_create(dir.path()).unwrap();
    push.notify_all(&json!({ "title": "t" })).await;
    assert!(!dir.path().join("push-subscriptions.json").exists());
}
