//! `POST /v1/realtime/sessions`: a JSON SDP offer in, a JSON SDP answer out.
use axum::body::to_bytes;
use axum::extract::{Request, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;

use super::{RealtimeClient, RealtimeClientConfig, RealtimeError, RealtimeOffer, MAX_SDP_BYTES};
use crate::server::{ApiError, AppState};

pub(crate) async fn create_session(
    State(state): State<AppState>,
    request: Request,
) -> Result<Response, ApiError> {
    state.authorize(request.headers())?;
    let is_json = request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next())
        .is_some_and(|v| v.trim() == "application/json");
    if !is_json {
        return Err(ApiError::bad_request(
            "realtime offer requires application/json",
        ));
    }
    let bytes = to_bytes(request.into_body(), MAX_SDP_BYTES * 2 + 4096)
        .await
        .map_err(|_| ApiError::payload_too_large("realtime request exceeds the size limit"))?;
    let offer: RealtimeOffer = serde_json::from_slice(&bytes)
        .map_err(|_| ApiError::bad_request("invalid realtime offer JSON or unsupported option"))?;
    offer.validate().map_err(realtime_error)?;
    // Held only while signaling; the media session is not a server resource.
    let _permit = state.limiter.clone().try_acquire_owned().map_err(|_| {
        ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "server is busy; try again shortly",
            "rate_limit_error",
        )
    })?;
    let config = &state.config;
    let auth = config.auth.load().map_err(|_| {
        ApiError::unauthorized("cannot load Codex authentication; renew the server's Codex login")
    })?;
    let client = RealtimeClient::new(
        auth,
        RealtimeClientConfig {
            upstream_base: config.upstream.realtime_base.clone(),
            proxy: config.upstream.proxy.clone(),
            timeout: config.upstream.request_timeout,
            connect_timeout: config.upstream.connect_timeout,
        },
    )
    .map_err(realtime_error)?;
    let answer = client.create_session(offer).await.map_err(realtime_error)?;
    Ok(Json(answer).into_response())
}

fn realtime_error(error: RealtimeError) -> ApiError {
    let (status, error_type) = match error {
        RealtimeError::InvalidOffer(_) => (StatusCode::BAD_REQUEST, "invalid_request_error"),
        RealtimeError::TooLarge => (StatusCode::PAYLOAD_TOO_LARGE, "realtime_size_limit"),
        RealtimeError::Challenge => (StatusCode::BAD_GATEWAY, "upstream_challenge"),
        RealtimeError::AuthRejected | RealtimeError::Authentication => {
            (StatusCode::BAD_GATEWAY, "upstream_authentication_error")
        }
        RealtimeError::RateLimited => (StatusCode::TOO_MANY_REQUESTS, "upstream_rate_limit"),
        RealtimeError::Timeout => (StatusCode::GATEWAY_TIMEOUT, "upstream_timeout"),
        _ => (StatusCode::BAD_GATEWAY, "upstream_error"),
    };
    ApiError::new(status, error.to_string(), error_type)
}
