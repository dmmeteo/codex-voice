//! HTTP composition: the public API routes, the static demo client, shared
//! limits, browser-origin guards, and the OpenAI-style error body.
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Serialize;
use tokio::net::TcpListener;
use tokio::sync::Semaphore;

use crate::audio::{self, SilkConfig};
use crate::auth::{self, AuthSource};

#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Local API key required as `Authorization: Bearer <key>`; `None` disables it.
    pub api_key: Option<String>,
    pub auth: AuthSource,
    pub upstream: UpstreamConfig,
    /// Upstream requests in flight at once, shared by both domains.
    pub concurrency: usize,
    pub max_upload_bytes: usize,
    pub silk: SilkConfig,
    pub default_language: Option<String>,
    /// Exact `host[:port]` authorities a reverse proxy presents for the
    /// browser routes, in addition to the bind address. Ignored without an
    /// API key, so a proxy can never widen keyless access.
    pub allowed_hosts: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct UpstreamConfig {
    pub transcribe_endpoint: String,
    #[cfg(feature = "realtime")]
    pub realtime_base: String,
    pub proxy: Option<String>,
    pub request_timeout: Duration,
    pub connect_timeout: Duration,
}

#[derive(Clone)]
pub(crate) struct AppState {
    pub config: Arc<ServerConfig>,
    pub limiter: Arc<Semaphore>,
    #[cfg(feature = "realtime")]
    pub live_limiter: Arc<Semaphore>,
    browser_hosts: Arc<[String]>,
}

impl AppState {
    pub fn authorize(&self, headers: &HeaderMap) -> Result<(), ApiError> {
        let authorization = headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok());
        if auth::is_authorized(authorization, self.config.api_key.as_deref()) {
            Ok(())
        } else {
            Err(ApiError::unauthorized("missing or invalid bearer token"))
        }
    }
}

/// Serves `listener` until Ctrl-C.
pub async fn serve(listener: TcpListener, config: ServerConfig) -> std::io::Result<()> {
    let addr = listener.local_addr()?;
    let max_upload_mib = config.max_upload_bytes / 1024 / 1024;
    let app = router(config, addr);
    eprintln!("codex-voice listening on http://{addr} (max upload: {max_upload_mib} MiB)");
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
}

/// The full application for a server reachable at `addr`.
pub fn router(config: ServerConfig, addr: SocketAddr) -> Router {
    let browser_hosts = browser_hosts(addr, config.api_key.is_some(), &config.allowed_hosts);
    if browser_hosts.is_empty() {
        eprintln!(
            "web client and realtime routes disabled: a non-loopback bind requires a local API key"
        );
    }
    let max_upload_bytes = config.max_upload_bytes;
    let state = AppState {
        limiter: Arc::new(Semaphore::new(config.concurrency)),
        #[cfg(feature = "realtime")]
        live_limiter: Arc::new(Semaphore::new(config.concurrency)),
        browser_hosts: browser_hosts.into(),
        config: Arc::new(config),
    };

    let api = Router::new().route(
        "/v1/audio/transcriptions",
        post(audio::routes::create_transcription).layer(DefaultBodyLimit::max(max_upload_bytes)),
    );
    let api = api.layer(middleware::from_fn(reject_cross_site));

    let browser = Router::new()
        .route("/", get(web::index))
        .route("/index.js", get(web::index_js))
        .route("/transcriptions.js", get(web::transcriptions_js))
        .route("/realtime.js", get(web::realtime_js))
        .route("/styles.css", get(web::styles));
    #[cfg(feature = "realtime")]
    let browser = browser
        .route(
            "/v1/realtime/live",
            get(crate::realtime::live_routes::connect),
        )
        .route(
            "/v1/realtime/sessions",
            post(crate::realtime::routes::create_session),
        );
    let browser = browser.layer(middleware::from_fn_with_state(
        state.clone(),
        guard_browser_origin,
    ));

    Router::new()
        .route("/healthz", get(healthz))
        .merge(api)
        .merge(browser)
        .with_state(state)
}

/// Host names accepted for browser-facing routes. Without a local API key,
/// only a loopback bind serves them, which blocks DNS rebinding to a public bind.
fn browser_hosts(addr: SocketAddr, has_api_key: bool, allowed_hosts: &[String]) -> Vec<String> {
    if !has_api_key && !addr.ip().is_loopback() {
        return Vec::new();
    }
    let mut hosts = vec![addr.to_string()];
    if has_api_key {
        hosts.extend(allowed_hosts.iter().cloned());
    }
    if addr.ip().is_loopback() || addr.ip().is_unspecified() {
        hosts.extend([
            format!("localhost:{}", addr.port()),
            format!("127.0.0.1:{}", addr.port()),
            format!("[::1]:{}", addr.port()),
        ]);
    }
    hosts
}

async fn healthz() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok" }))
}

async fn guard_browser_origin(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    check_host(request.headers(), &state.browser_hosts)?;
    check_same_origin(request.headers())?;
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static("default-src 'self'; script-src 'self'; style-src 'self'; connect-src 'self'; media-src 'self' blob:; frame-ancestors 'none'; base-uri 'none'; form-action 'none'"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    Ok(response)
}

/// API clients send no `Origin`; a browser on another site must not spend
/// the server's Codex login through a simple multipart POST.
async fn reject_cross_site(request: Request, next: Next) -> Result<Response, ApiError> {
    check_same_origin(request.headers())?;
    Ok(next.run(request).await)
}

fn forbidden() -> ApiError {
    ApiError::new(
        StatusCode::FORBIDDEN,
        "browser requests must use the configured host and same origin",
        "origin_error",
    )
}

fn check_host(headers: &HeaderMap, allowed_hosts: &[String]) -> Result<(), ApiError> {
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(forbidden)?;
    if allowed_hosts
        .iter()
        .any(|allowed| allowed.eq_ignore_ascii_case(host))
    {
        Ok(())
    } else {
        Err(forbidden())
    }
}

fn check_same_origin(headers: &HeaderMap) -> Result<(), ApiError> {
    if let Some(origin) = headers.get(header::ORIGIN) {
        let host = headers
            .get(header::HOST)
            .and_then(|v| v.to_str().ok())
            .ok_or_else(forbidden)?;
        let origin = origin
            .to_str()
            .ok()
            .and_then(|v| reqwest::Url::parse(v).ok())
            .ok_or_else(forbidden)?;
        if !matches!(origin.scheme(), "http" | "https")
            || !origin.authority().eq_ignore_ascii_case(host)
            || origin.path() != "/"
            || origin.query().is_some()
            || origin.fragment().is_some()
            || !origin.username().is_empty()
            || origin.password().is_some()
        {
            return Err(forbidden());
        }
    }
    if headers
        .get("sec-fetch-site")
        .is_some_and(|v| v != "same-origin" && v != "none")
    {
        return Err(forbidden());
    }
    Ok(())
}

/// The static demonstration client, compiled into the binary.
mod web {
    use axum::http::header;
    use axum::response::{Html, IntoResponse};

    const JS: &str = "text/javascript; charset=utf-8";

    pub async fn index() -> Html<&'static str> {
        Html(include_str!("../web/index.html"))
    }
    pub async fn index_js() -> impl IntoResponse {
        (
            [(header::CONTENT_TYPE, JS)],
            include_str!("../web/index.js"),
        )
    }
    pub async fn transcriptions_js() -> impl IntoResponse {
        (
            [(header::CONTENT_TYPE, JS)],
            include_str!("../web/transcriptions.js"),
        )
    }
    pub async fn realtime_js() -> impl IntoResponse {
        (
            [(header::CONTENT_TYPE, JS)],
            include_str!("../web/realtime.js"),
        )
    }
    pub async fn styles() -> impl IntoResponse {
        (
            [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
            include_str!("../web/styles.css"),
        )
    }
}

#[derive(Debug)]
pub(crate) struct ApiError {
    status: StatusCode,
    message: String,
    error_type: &'static str,
}

impl ApiError {
    pub fn new(status: StatusCode, message: impl Into<String>, error_type: &'static str) -> Self {
        Self {
            status,
            message: message.into(),
            error_type,
        }
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message, "invalid_request_error")
    }

    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, message, "authentication_error")
    }

    pub fn payload_too_large(message: impl Into<String>) -> Self {
        Self::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            message,
            "invalid_request_error",
        )
    }

    pub fn bad_gateway(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_GATEWAY, message, "upstream_error")
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, message, "server_error")
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        #[derive(Serialize)]
        struct ErrorBody {
            error: ErrorPayload,
        }
        #[derive(Serialize)]
        struct ErrorPayload {
            message: String,
            #[serde(rename = "type")]
            error_type: &'static str,
            param: Option<String>,
            code: Option<String>,
        }
        let body = ErrorBody {
            error: ErrorPayload {
                message: self.message,
                error_type: self.error_type,
                param: None,
                code: None,
            },
        };
        (self.status, Json(body)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyless_browser_routes_require_a_loopback_bind() {
        for host in ["0.0.0.0", "[::]", "192.0.2.1", "[2001:db8::1]"] {
            let addr = format!("{host}:2600").parse().unwrap();
            assert!(browser_hosts(addr, false, &[]).is_empty(), "{host}");
            assert!(!browser_hosts(addr, true, &[]).is_empty(), "{host}");
        }
        for host in ["127.0.0.1", "127.0.0.2", "[::1]"] {
            let addr = format!("{host}:2600").parse().unwrap();
            assert!(browser_hosts(addr, false, &[]).contains(&addr.to_string()));
        }
    }

    #[test]
    fn allowed_hosts_apply_only_with_an_api_key() {
        let proxy = vec!["192.168.1.100".to_string()];
        let addr = "172.19.0.1:2620".parse().unwrap();
        assert!(browser_hosts(addr, false, &proxy).is_empty());
        assert_eq!(
            browser_hosts(addr, true, &proxy),
            vec!["172.19.0.1:2620".to_string(), "192.168.1.100".to_string()]
        );
        let loopback = "127.0.0.1:2600".parse().unwrap();
        assert!(!browser_hosts(loopback, false, &proxy).contains(&proxy[0]));
    }

    #[test]
    fn host_and_origin_reject_cross_site_and_rebinding() {
        let mut headers = HeaderMap::new();
        let hosts = vec!["127.0.0.1:2600".to_string()];
        let check = |headers: &HeaderMap| {
            check_host(headers, &hosts).and_then(|()| check_same_origin(headers))
        };
        headers.insert(header::HOST, "127.0.0.1:2600".parse().unwrap());
        assert!(check(&headers).is_ok());
        headers.insert(header::ORIGIN, "http://127.0.0.1:2600".parse().unwrap());
        assert!(check(&headers).is_ok());
        headers.insert(header::ORIGIN, "https://attacker.example".parse().unwrap());
        assert!(check(&headers).is_err());
        headers.remove(header::ORIGIN);
        headers.insert(header::HOST, "rebind.example:2600".parse().unwrap());
        assert!(check(&headers).is_err());
        headers.insert(header::HOST, "127.0.0.1:2600".parse().unwrap());
        headers.insert("sec-fetch-site", "cross-site".parse().unwrap());
        assert!(check(&headers).is_err());
    }
}
