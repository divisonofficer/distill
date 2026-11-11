//! Router for untrusted content on a separate origin: local port proxy and session file serving.
//! Public API: plan/research-base/CONTRACT.md §5.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::{Path as AxumPath, Request, State};
use axum::http::{HeaderMap, HeaderName, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get};

/// Session id -> session cwd.
pub type CwdResolver = Arc<dyn Fn(&str) -> Option<PathBuf> + Send + Sync>;

#[derive(Clone)]
struct Edge {
    resolve_cwd: CwdResolver,
    client: reqwest::Client,
}

const HOP_BY_HOP: [&str; 9] = [
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "proxy-connection",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

pub fn routes(resolve_cwd: CwdResolver) -> Router {
    // Loopback-only upstream: no env proxy, no redirect following (the client gets the upstream reply as is).
    #[allow(
        clippy::disallowed_methods,
        reason = "localhost-only proxy client to 127.0.0.1 ports; no TLS"
    )]
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());
    let state = Edge {
        resolve_cwd,
        client,
    };
    Router::new()
        .route("/p/{port}", any(proxy))
        .route("/p/{port}/", any(proxy))
        .route("/p/{port}/{*rest}", any(proxy))
        .route("/f/{session_id}/{*path}", get(serve_file))
        .with_state(state)
}

/// Forward `/p/{port}/rest` to `http://127.0.0.1:{port}/rest`, streaming body and reply.
async fn proxy(State(edge): State<Edge>, req: Request) -> Response {
    let (parts, body) = req.into_parts();

    // Raw (undecoded) path: `/p/{port}` followed by `/rest`, if any.
    let mut segments = parts.uri.path().splitn(4, '/');
    let port = segments.nth(2).unwrap_or_default();
    let after = format!("/{}", segments.next().unwrap_or_default());
    let port: u16 = match port.parse() {
        Ok(p) if p != 0 => p,
        _ => return (StatusCode::BAD_REQUEST, "invalid port").into_response(),
    };
    if parts.headers.contains_key(header::UPGRADE) {
        return (StatusCode::NOT_IMPLEMENTED, "websocket proxy not supported").into_response();
    }

    let mut url = format!("http://127.0.0.1:{port}{after}");
    if let Some(query) = parts.uri.query() {
        url.push('?');
        url.push_str(query);
    }

    let upstream = edge
        .client
        .request(parts.method.clone(), url)
        .headers(forward_headers(&parts.headers))
        .body(reqwest::Body::wrap_stream(body.into_data_stream()))
        .send()
        .await;
    let upstream = match upstream {
        Ok(res) => res,
        Err(_) => return (StatusCode::BAD_GATEWAY, "upstream unreachable").into_response(),
    };

    let status = upstream.status();
    let headers = forward_headers(upstream.headers());
    let mut res = Response::new(Body::from_stream(upstream.bytes_stream()));
    *res.status_mut() = status;
    *res.headers_mut() = headers;
    res
}

/// Copy headers minus hop-by-hop, `Host`, `Content-Length` (reqwest sets it) and `Tailscale-*`.
fn forward_headers(src: &HeaderMap) -> HeaderMap {
    let mut out = HeaderMap::new();
    for (name, value) in src {
        if is_dropped(name) {
            continue;
        }
        out.append(name.clone(), value.clone());
    }
    out
}

fn is_dropped(name: &HeaderName) -> bool {
    let n = name.as_str();
    n == "host" || n == "content-length" || n.starts_with("tailscale-") || HOP_BY_HOP.contains(&n)
}

/// Serve `path` under the session cwd. Anything outside it, or not a regular file, is 404.
async fn serve_file(
    State(edge): State<Edge>,
    AxumPath((session_id, path)): AxumPath<(String, String)>,
) -> Response {
    let not_found = || StatusCode::NOT_FOUND.into_response();

    let Some(cwd) = (edge.resolve_cwd)(&session_id) else {
        return not_found();
    };
    let rel = Path::new(&path);
    if rel.components().any(|c| !matches!(c, Component::Normal(_))) {
        return not_found();
    }
    let Ok(root) = dunce::canonicalize(&cwd) else {
        return not_found();
    };
    let Ok(file) = dunce::canonicalize(root.join(rel)) else {
        return not_found();
    };
    if !file.starts_with(&root) {
        return not_found();
    }
    let Ok(meta) = tokio::fs::metadata(&file).await else {
        return not_found();
    };
    if !meta.is_file() {
        return not_found();
    }
    let Ok(bytes) = tokio::fs::read(&file).await else {
        return not_found();
    };
    let mime = content_type(&file);
    ([(header::CONTENT_TYPE, mime)], bytes).into_response()
}

fn content_type(path: &Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    match ext.as_deref() {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("svg") => "image/svg+xml",
        Some("json") => "application/json",
        Some("txt") => "text/plain; charset=utf-8",
        Some("md") => "text/markdown; charset=utf-8",
        _ => "application/octet-stream",
    }
}
