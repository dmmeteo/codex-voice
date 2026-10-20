//! HTTP composition: the public transcription route, shared limits, the
//! cross-site guard, and the OpenAI-style error body.
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{DefaultBodyLimit, Request};
use axum::http::{header, HeaderMap, StatusCode};
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
}

#[derive(Debug, Clone)]
pub struct UpstreamConfig {
    pub transcribe_endpoint: String,
    pub proxy: Option<String>,
    pub request_timeout: Duration,
    pub connect_timeout: Duration,
}

#[derive(Clone)]
pub(crate) struct AppState {
    pub config: Arc<ServerConfig>,
    pub limiter: Arc<Semaphore>,
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
    let app = router(config);
    eprintln!("codex-voice listening on http://{addr} (max upload: {max_upload_mib} MiB)");
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
}

/// The full application.
pub fn router(config: ServerConfig) -> Router {
    let max_upload_bytes = config.max_upload_bytes;
    let state = AppState {
        limiter: Arc::new(Semaphore::new(config.concurrency)),
        config: Arc::new(config),
    };

    let api = Router::new().route(
        "/v1/audio/transcriptions",
        post(audio::routes::create_transcription).layer(DefaultBodyLimit::max(max_upload_bytes)),
    );
    let api = api.layer(middleware::from_fn(reject_cross_site));

    Router::new()
        .route("/healthz", get(healthz))
        .merge(api)
        .with_state(state)
}

async fn healthz() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok" }))
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
    fn cross_site_browser_requests_are_rejected() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, "127.0.0.1:2600".parse().unwrap());
        assert!(check_same_origin(&headers).is_ok());
        headers.insert(header::ORIGIN, "http://127.0.0.1:2600".parse().unwrap());
        assert!(check_same_origin(&headers).is_ok());
        headers.insert(header::ORIGIN, "https://attacker.example".parse().unwrap());
        assert!(check_same_origin(&headers).is_err());
        headers.remove(header::ORIGIN);
        headers.insert("sec-fetch-site", "cross-site".parse().unwrap());
        assert!(check_same_origin(&headers).is_err());
    }
}
