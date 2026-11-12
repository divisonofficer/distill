//! Router for untrusted content on a separate origin: local port proxy and session file serving.
//! Public API: plan/research-base/CONTRACT.md §5.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::{FromRequestParts, Path as AxumPath, Request, State, WebSocketUpgrade};
use axum::http::{HeaderMap, HeaderName, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;

/// Session id -> session cwd.
pub type CwdResolver = Arc<dyn Fn(&str) -> Option<PathBuf> + Send + Sync>;

#[derive(Clone)]
struct Edge {
    resolve_cwd: CwdResolver,
    client: reqwest::Client,
    prohibited_ports: Arc<[u16]>,
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
    routes_with_prohibited_ports(resolve_cwd, [])
}

/// Build untrusted routes while denying proxy access to node-owned TCP ports.
pub fn routes_with_prohibited_ports(
    resolve_cwd: CwdResolver,
    prohibited_ports: impl IntoIterator<Item = u16>,
) -> Router {
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
        prohibited_ports: prohibited_ports.into_iter().collect::<Vec<_>>().into(),
    };
    Router::new()
        .route("/p/{port}", any(proxy))
        .route("/p/{port}/", any(proxy))
        .route("/p/{port}/{*rest}", any(proxy))
        .route("/f/{session_id}/{*path}", get(serve_file))
        .with_state(state)
}

/// Forward `/p/{port}/rest` to `http://127.0.0.1:{port}/rest`.
async fn proxy(State(edge): State<Edge>, req: Request) -> Response {
    let (mut parts, body) = req.into_parts();
    let (port, after) = match proxy_target(&parts.uri) {
        Ok(target) => target,
        Err(status) => return (status, "invalid port").into_response(),
    };
    if edge.prohibited_ports.contains(&port) {
        return (StatusCode::FORBIDDEN, "port is not proxyable").into_response();
    }
    if is_websocket(&parts.headers) {
        let ws = match WebSocketUpgrade::from_request_parts(&mut parts, &()).await {
            Ok(ws) => ws,
            Err(rejection) => return rejection.into_response(),
        };
        let mut upstream = format!("ws://127.0.0.1:{port}{after}");
        if let Some(query) = parts.uri.query() {
            upstream.push('?');
            upstream.push_str(query);
        }
        let headers = forward_headers(&parts.headers);
        let upstream = match connect_websocket(&upstream, &headers).await {
            Ok(upstream) => upstream,
            Err(_) => {
                return (StatusCode::BAD_GATEWAY, "upstream websocket unavailable").into_response();
            }
        };
        let (upstream_socket, protocol) = upstream;
        let protocols = protocol.into_iter().collect::<Vec<_>>();
        let ws = ws.protocols(protocols);
        return ws.on_upgrade(move |socket| websocket_tunnel(socket, upstream_socket));
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

fn proxy_target(uri: &axum::http::Uri) -> Result<(u16, String), StatusCode> {
    let mut segments = uri.path().splitn(4, '/');
    let port = segments.nth(2).unwrap_or_default();
    let after = format!("/{}", segments.next().unwrap_or_default());
    let port = port.parse::<u16>().map_err(|_| StatusCode::BAD_REQUEST)?;
    if port == 0 {
        return Err(StatusCode::BAD_REQUEST);
    }
    Ok((port, after))
}

fn is_websocket(headers: &HeaderMap) -> bool {
    headers
        .get(header::UPGRADE)
        .is_some_and(|value| value.as_bytes().eq_ignore_ascii_case(b"websocket"))
}

async fn connect_websocket(
    upstream_url: &str,
    headers: &HeaderMap,
) -> Result<
    (
        tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
        Option<String>,
    ),
    (),
> {
    let Some(port) = upstream_url
        .strip_prefix("ws://127.0.0.1:")
        .and_then(|s| s.split('/').next())
        .and_then(|s| s.parse::<u16>().ok())
    else {
        return Err(());
    };
    let stream = tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port))
        .await
        .map_err(|_| ())?;
    let mut request =
        tokio_tungstenite::tungstenite::client::IntoClientRequest::into_client_request(
            upstream_url,
        )
        .map_err(|_| ())?;
    for (name, value) in headers {
        if name != header::HOST
            && name != header::CONNECTION
            && name != header::UPGRADE
            && !name.as_str().starts_with("sec-websocket-")
            && name != header::SEC_WEBSOCKET_PROTOCOL
        {
            request.headers_mut().append(name.clone(), value.clone());
        }
    }
    if let Some(protocols) = headers.get(header::SEC_WEBSOCKET_PROTOCOL) {
        request
            .headers_mut()
            .insert(header::SEC_WEBSOCKET_PROTOCOL, protocols.clone());
    }
    let (upstream, response) = tokio_tungstenite::client_async(request, stream)
        .await
        .map_err(|_| ())?;
    let selected = response
        .headers()
        .get(header::SEC_WEBSOCKET_PROTOCOL)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    Ok((upstream, selected))
}

async fn websocket_tunnel(
    downstream: axum::extract::ws::WebSocket,
    upstream: tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
) {
    let (mut down_tx, mut down_rx) = downstream.split();
    let (mut up_tx, mut up_rx) = upstream.split();
    let downstream_to_upstream = async {
        while let Some(message) = down_rx.next().await {
            match message {
                Ok(message) => {
                    let message = match message {
                        axum::extract::ws::Message::Text(text) => {
                            Message::Text(text.as_str().into())
                        }
                        axum::extract::ws::Message::Binary(data) => Message::Binary(data),
                        axum::extract::ws::Message::Ping(data) => Message::Ping(data),
                        axum::extract::ws::Message::Pong(data) => Message::Pong(data),
                        axum::extract::ws::Message::Close(frame) => {
                            let frame = frame.map(|f| {
                                tokio_tungstenite::tungstenite::protocol::CloseFrame {
                                    code: f.code.into(),
                                    reason: f.reason.to_string().into(),
                                }
                            });
                            let _ = up_tx.send(Message::Close(frame)).await;
                            break;
                        }
                    };
                    if up_tx.send(message).await.is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    };
    let upstream_to_downstream = async {
        while let Some(message) = up_rx.next().await {
            match message {
                Ok(Message::Text(text)) => {
                    if down_tx
                        .send(axum::extract::ws::Message::Text(text.to_string().into()))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Ok(Message::Binary(data)) => {
                    if down_tx
                        .send(axum::extract::ws::Message::Binary(data))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Ok(Message::Ping(data)) => {
                    if down_tx
                        .send(axum::extract::ws::Message::Ping(data))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Ok(Message::Pong(data)) => {
                    if down_tx
                        .send(axum::extract::ws::Message::Pong(data))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Ok(Message::Close(frame)) => {
                    let frame = frame.map(|f| axum::extract::ws::CloseFrame {
                        code: f.code.into(),
                        reason: f.reason.to_string().into(),
                    });
                    let _ = down_tx.send(axum::extract::ws::Message::Close(frame)).await;
                    break;
                }
                Ok(Message::Frame(_)) => {}
                Err(_) => break,
            }
        }
    };
    tokio::select! {
        _ = downstream_to_upstream => {},
        _ = upstream_to_downstream => {},
    }
}

/// Copy headers minus hop-by-hop, `Host`, `Content-Length` and `Tailscale-*`.
fn forward_headers(src: &HeaderMap) -> HeaderMap {
    let nominated = connection_tokens(src);
    let mut out = HeaderMap::new();
    for (name, value) in src {
        if is_dropped(name) || nominated.contains(name.as_str()) {
            continue;
        }
        out.append(name.clone(), value.clone());
    }
    out
}

fn connection_tokens(headers: &HeaderMap) -> std::collections::HashSet<String> {
    headers
        .get_all(header::CONNECTION)
        .iter()
        .flat_map(|value| value.to_str().unwrap_or_default().split(','))
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
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
