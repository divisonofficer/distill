use crate::research::ResearchService;
use axum::{
    Router,
    body::Body,
    extract::{Request, State},
    http::{HeaderValue, Method, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use ring::rand::{SecureRandom, SystemRandom};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;
use url::Url;

const TICKET_TTL: Duration = Duration::from_secs(30);
const SESSION_TTL: Duration = Duration::from_secs(1800);
#[derive(Clone)]
pub(crate) struct DashboardGateway {
    origin: Url,
    service: ResearchService,
    tickets: Arc<Mutex<HashMap<String, (String, Instant)>>>,
    sessions: Arc<Mutex<HashMap<String, (String, Instant)>>>,
}
impl DashboardGateway {
    pub(crate) fn new(origin: Url, service: ResearchService) -> anyhow::Result<Self> {
        anyhow::ensure!(
            origin.scheme() == "https"
                && origin.host_str().is_some()
                && origin.username().is_empty()
                && origin.password().is_none()
                && origin.path() == "/"
                && origin.query().is_none()
                && origin.fragment().is_none(),
            "Invalid dashboard origin"
        );
        Ok(Self {
            origin,
            service,
            tickets: Default::default(),
            sessions: Default::default(),
        })
    }
    pub(crate) async fn ticket(&self, project: &str) -> Result<String, Response> {
        self.service.owned_project(project).await?;
        let token = random_token();
        let mut tickets = self.tickets.lock().await;
        tickets.retain(|_, (_, expiry)| *expiry > Instant::now());
        if tickets.len() >= 1024 {
            return Err(StatusCode::TOO_MANY_REQUESTS.into_response());
        }
        tickets.insert(
            token.clone(),
            (project.to_owned(), Instant::now() + TICKET_TTL),
        );
        Ok(format!("{}exchange?ticket={token}", self.origin))
    }
    pub(crate) fn router(&self) -> Router {
        Router::new()
            .route("/exchange", get(exchange))
            .route("/", get(proxy))
            .route("/{*path}", get(proxy))
            .with_state(self.clone())
    }
    fn cookie_name(&self) -> &'static str {
        if self.origin.scheme() == "https" {
            "__Host-postudio_dashboard"
        } else {
            "postudio_dashboard"
        }
    }
    fn valid_request(&self, request: &Request) -> bool {
        let authority = self.origin[url::Position::BeforeHost..url::Position::AfterPort].to_owned();
        request
            .headers()
            .get(header::HOST)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.eq_ignore_ascii_case(&authority))
            && request.headers().get(header::ORIGIN).is_none_or(|v| {
                v.to_str()
                    .ok()
                    .is_some_and(|v| v == self.origin.origin().ascii_serialization())
            })
            && request.headers().get(header::UPGRADE).is_none()
            && request.method() == Method::GET
    }
    fn session_token(&self, request: &Request) -> Option<String> {
        if !self.valid_request(request) {
            return None;
        }
        let token = request
            .headers()
            .get(header::COOKIE)?
            .to_str()
            .ok()?
            .split(';')
            .find_map(|part| {
                part.trim()
                    .strip_prefix(&format!("{}=", self.cookie_name()))
            })?
            .to_owned();
        Some(token)
    }
    async fn project(&self, token: String) -> Option<String> {
        let mut sessions = self.sessions.lock().await;
        sessions.retain(|_, (_, expiry)| *expiry > Instant::now());
        sessions.get(&token).map(|(project, _)| project.clone())
    }
}
fn random_token() -> String {
    let mut bytes = [0u8; 32];
    SystemRandom::new()
        .fill(&mut bytes)
        .expect("operating system randomness");
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
async fn exchange(State(g): State<DashboardGateway>, request: Request) -> Response {
    if !g.valid_request(&request) {
        return secured(StatusCode::FORBIDDEN.into_response());
    }
    let ticket = request
        .uri()
        .query()
        .and_then(|q| q.strip_prefix("ticket="))
        .filter(|t| t.len() == 64 && t.bytes().all(|b| b.is_ascii_hexdigit()));
    let entry = match ticket {
        Some(ticket) => g.tickets.lock().await.remove(ticket),
        None => None,
    };
    let Some((project, expiry)) = entry.filter(|(_, expiry)| *expiry > Instant::now()) else {
        return secured(StatusCode::UNAUTHORIZED.into_response());
    };
    let _ = expiry;
    let token = random_token();
    let mut sessions = g.sessions.lock().await;
    sessions.retain(|_, (_, expiry)| *expiry > Instant::now());
    if sessions.len() >= 1024 {
        return secured(StatusCode::TOO_MANY_REQUESTS.into_response());
    }
    sessions.insert(
        token.clone(),
        (project.clone(), Instant::now() + SESSION_TTL),
    );
    let mut response = StatusCode::SEE_OTHER.into_response();
    response.headers_mut().insert(
        header::LOCATION,
        HeaderValue::from_str(&format!("/projects/{}", urlencoding::encode(&project)))
            .expect("encoded project"),
    );
    let secure = if g.origin.scheme() == "https" {
        "; Secure"
    } else {
        ""
    };
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&format!(
            "{}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age=1800{secure}",
            g.cookie_name()
        ))
        .expect("hex cookie"),
    );
    secured(response)
}
async fn proxy(State(g): State<DashboardGateway>, request: Request) -> Response {
    let Some(token) = g.session_token(&request) else {
        return secured(StatusCode::UNAUTHORIZED.into_response());
    };
    let Some(project) = g.project(token).await else {
        return secured(StatusCode::UNAUTHORIZED.into_response());
    };
    let path = request.uri().path();
    if path.contains('%') || path.contains('\\') || path.contains("..") {
        return secured(StatusCode::FORBIDDEN.into_response());
    }
    let parts: Vec<_> = path.trim_matches('/').split('/').collect();
    if path == "/api/projects" {
        return secured(match g.service.owned_project(&project).await {
            Ok(value) => {
                axum::Json(serde_json::json!({"projects":[value.get("project").unwrap_or(&value)]}))
                    .into_response()
            }
            Err(error) => error,
        });
    }
    let project_read = parts.len() >= 3
        && parts[0] == "api"
        && parts[1] == "projects"
        && parts[2] == project
        && (parts.len() == 3
            || (parts.len() == 4
                && matches!(
                    parts[3],
                    "experiments" | "runs" | "files" | "git" | "code-tree"
                )));
    let run_read = parts.len() >= 3
        && parts[0] == "api"
        && parts[1] == "runs"
        && (parts.len() == 3 || (parts.len() == 4 && matches!(parts[3], "logs" | "diff")));
    let experiment_read = parts.len() == 4
        && parts[0] == "api"
        && parts[1] == "experiments"
        && matches!(parts[3], "diff" | "commits");
    if run_read {
        if let Err(error) = g.service.owned_run(&project, parts[2]).await {
            return secured(error);
        }
    } else if experiment_read
        && let Err(error) = g.service.owned_experiment(&project, parts[2]).await
    {
        return secured(error);
    }
    let asset = path.starts_with("/assets/")
        && !path.ends_with(".html")
        && !path.ends_with(".svg")
        && matches!(
            path.rsplit('.').next(),
            Some("js" | "css" | "woff" | "woff2" | "png" | "jpg" | "ico")
        );
    let spa = path == "/" || path == "/projects" || path == format!("/projects/{project}");
    if !(project_read || run_read || experiment_read || asset || spa) {
        return secured(StatusCode::FORBIDDEN.into_response());
    }
    // Only native static assets and audited read-only routes reach the upstream service.
    let native_path = if spa {
        "".to_owned()
    } else {
        request
            .uri()
            .path_and_query()
            .map(|p| p.as_str().trim_start_matches('/').to_owned())
            .unwrap_or_default()
    };
    let limit = if asset || spa {
        16 * 1024 * 1024
    } else {
        1024 * 1024
    };
    let mut response = match g.service.native_bytes(&native_path, limit).await {
        Ok((content_type, mut bytes)) => {
            if spa {
                let Ok(html) = String::from_utf8(bytes) else {
                    return secured(StatusCode::BAD_GATEWAY.into_response());
                };
                bytes = html.replace("<body>", "<body><aside role=note style=\"padding:8px;background:#fdf3e3;color:#8a4a00\">Postudio 읽기 전용 대시보드</aside>").into_bytes();
            }
            let mut response = Body::from(bytes).into_response();
            if let Ok(value) = HeaderValue::from_str(&content_type) {
                response.headers_mut().insert(header::CONTENT_TYPE, value);
            }
            response
        }
        Err(error) => error,
    };
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    secured(response)
}
fn secured(mut response: Response) -> Response {
    for (name, value) in [
        (header::X_FRAME_OPTIONS, "DENY"),
        (header::REFERRER_POLICY, "no-referrer"),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        (
            header::CONTENT_SECURITY_POLICY,
            "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; object-src 'none'; base-uri 'none'; form-action 'none'",
        ),
        (header::CACHE_CONTROL, "no-store"),
    ] {
        response
            .headers_mut()
            .insert(name, HeaderValue::from_static(value));
    }
    response
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tokens_are_unique_hex_capabilities() {
        let first = random_token();
        assert_eq!(first.len(), 64);
        assert!(first.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_ne!(first, random_token());
    }
    #[test]
    fn every_error_gets_security_headers() {
        let response = secured(StatusCode::FORBIDDEN.into_response());
        assert_eq!(response.headers()[header::X_FRAME_OPTIONS], "DENY");
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    }
}
