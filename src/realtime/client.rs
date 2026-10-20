//! WebRTC session signaling with ChatGPT's private voice service. This is not
//! the public OpenAI Realtime API. The caller owns WebRTC media and its lifecycle.
use std::time::Duration;

use futures_util::StreamExt;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;
use uuid::Uuid;
use wreq::{header, multipart, Client};

use crate::auth::{desktop_user_agent, CodexAuth, ORIGINATOR};

pub const MAX_SDP_BYTES: usize = 128 * 1024;
pub const DEFAULT_VOICE: &str = "cove";
pub const DEFAULT_UPSTREAM: &str = "https://chatgpt.com";

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RealtimeMode {
    Standard,
    #[default]
    Advanced,
    Wingman,
}

impl RealtimeMode {
    fn path(self) -> &'static str {
        match self {
            Self::Standard => "/realtime/vps",
            Self::Advanced => "/realtime/vp",
            Self::Wingman => "/realtime/wm",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RealtimeOffer {
    pub sdp: String,
    #[serde(default)]
    pub mode: RealtimeMode,
    #[serde(default = "default_voice")]
    pub voice: String,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default = "default_message_streaming")]
    pub enable_message_streaming: bool,
    #[serde(default = "default_history_disabled")]
    pub history_and_training_disabled: bool,
    #[serde(default = "default_timezone")]
    pub timezone: String,
    /// JavaScript getTimezoneOffset convention: minutes west of UTC.
    #[serde(default)]
    pub timezone_offset_minutes: i16,
}

fn default_voice() -> String {
    DEFAULT_VOICE.into()
}
fn default_timezone() -> String {
    "UTC".into()
}
fn default_message_streaming() -> bool {
    true
}
fn default_history_disabled() -> bool {
    true
}

impl RealtimeOffer {
    pub fn new(sdp: String) -> Self {
        Self {
            sdp,
            mode: RealtimeMode::default(),
            voice: default_voice(),
            language: None,
            enable_message_streaming: true,
            history_and_training_disabled: true,
            timezone: default_timezone(),
            timezone_offset_minutes: 0,
        }
    }

    pub fn validate(&self) -> Result<(), RealtimeError> {
        validate_sdp(&self.sdp)?;
        if self.voice.is_empty()
            || self.voice.len() > 64
            || !self
                .voice
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        {
            return Err(RealtimeError::InvalidOffer(
                "voice must be an identifier of 1 to 64 characters",
            ));
        }
        if self.language.as_ref().is_some_and(|v| {
            v.is_empty()
                || v.len() > 35
                || !v.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
        }) {
            return Err(RealtimeError::InvalidOffer(
                "language must be a language tag, for example en or uk-UA",
            ));
        }
        if self.timezone.is_empty()
            || self.timezone.len() > 64
            || !self
                .timezone
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"/_+-".contains(&c))
            || !(-840..=840).contains(&self.timezone_offset_minutes)
        {
            return Err(RealtimeError::InvalidOffer(
                "invalid timezone or UTC offset",
            ));
        }
        Ok(())
    }
}

fn validate_sdp(sdp: &str) -> Result<(), RealtimeError> {
    if sdp.len() > MAX_SDP_BYTES {
        return Err(RealtimeError::TooLarge);
    }
    if !sdp.starts_with("v=0\r\n") && !sdp.starts_with("v=0\n")
        || sdp.contains('\0')
        || !sdp.lines().any(|l| l.starts_with("m=audio "))
    {
        return Err(RealtimeError::InvalidOffer(
            "SDP must begin with v=0 and contain an audio media section",
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RealtimeAnswer {
    pub sdp: String,
    /// Correlation ID sent upstream. This is not a server-owned media resource.
    pub session_id: String,
}

#[derive(Debug, Error)]
pub enum RealtimeError {
    #[error("{0}")]
    InvalidOffer(&'static str),
    #[error("voice SDP or response exceeds the size limit")]
    TooLarge,
    #[error("invalid voice client configuration")]
    Configuration,
    #[error("invalid Codex authentication headers")]
    Authentication,
    #[error("voice service request timed out")]
    Timeout,
    #[error("could not connect to the voice service")]
    Transport,
    #[error(
        "ChatGPT returned a browser verification challenge; voice signaling was not established"
    )]
    Challenge,
    #[error("ChatGPT rejected voice authentication; renew your Codex login")]
    AuthRejected,
    #[error("ChatGPT voice usage or request limit reached")]
    RateLimited,
    #[error("ChatGPT rejected the voice session (HTTP {status})")]
    Upstream { status: u16 },
    #[error("ChatGPT returned an invalid voice SDP answer")]
    InvalidAnswer,
    #[error("ChatGPT returned an invalid default model for Wingman voice")]
    InvalidModelCatalog,
}

#[derive(Deserialize)]
struct ModelCatalog {
    default_model_slug: String,
}

#[derive(Debug, Clone)]
pub struct RealtimeClientConfig {
    /// Server-owned configuration, never taken from a browser offer.
    pub upstream_base: String,
    pub proxy: Option<String>,
    pub timeout: Duration,
    pub connect_timeout: Duration,
}

impl Default for RealtimeClientConfig {
    fn default() -> Self {
        Self {
            upstream_base: DEFAULT_UPSTREAM.into(),
            proxy: crate::resolve_proxy(None),
            timeout: Duration::from_secs(60),
            connect_timeout: Duration::from_secs(15),
        }
    }
}

#[derive(Clone)]
pub struct RealtimeClient {
    http: Client,
    auth: CodexAuth,
    upstream: Url,
}

impl RealtimeClient {
    pub fn new(auth: CodexAuth, config: RealtimeClientConfig) -> Result<Self, RealtimeError> {
        let upstream =
            Url::parse(&config.upstream_base).map_err(|_| RealtimeError::Configuration)?;
        if !matches!(upstream.scheme(), "http" | "https")
            || upstream.host_str().is_none()
            || !upstream.username().is_empty()
            || upstream.password().is_some()
            || upstream.query().is_some()
            || upstream.fragment().is_some()
            || upstream.path() != "/"
        {
            return Err(RealtimeError::Configuration);
        }
        let mut builder = Client::builder()
            .emulation(wreq_util::Emulation::Chrome136)
            .timeout(config.timeout)
            .connect_timeout(config.connect_timeout)
            .redirect(wreq::redirect::Policy::none());
        if let Some(proxy) = config.proxy {
            builder =
                builder.proxy(wreq::Proxy::all(proxy).map_err(|_| RealtimeError::Configuration)?);
        }
        Ok(Self {
            http: builder.build().map_err(|_| RealtimeError::Configuration)?,
            auth,
            upstream,
        })
    }

    pub async fn create_session(
        &self,
        offer: RealtimeOffer,
    ) -> Result<RealtimeAnswer, RealtimeError> {
        offer.validate()?;
        let session_id = Uuid::new_v4().to_string();
        let headers = self.auth_headers()?;
        let mut status_url = self.upstream.clone();
        status_url.set_path("/realtime/status");
        let status_response = self
            .http
            .post(status_url.as_str())
            .headers(headers.clone())
            .json(&json!({}))
            .send()
            .await
            .map_err(transport_error)?;
        let status: serde_json::Value =
            serde_json::from_str(&read_response(status_response).await?)
                .map_err(|_| RealtimeError::InvalidAnswer)?;
        let mut session = json!({
            "voice_session_id": session_id,
            "voice_status_request_id": session_id,
            "voice": offer.voice,
            "language_code": offer.language,
            "timezone_offset_min": offer.timezone_offset_minutes,
            "timezone": offer.timezone,
            "user_locale": offer.language.as_deref().unwrap_or("en-US"),
            "voice_mode": offer.mode,
            "model_slug": "gpt-4o",
            "requested_default_model": "gpt-4o",
            "backend_model": "gpt-4o",
            "conversation_mode": {"kind": "primary_assistant"},
            "history_and_training_disabled": offer.history_and_training_disabled,
            "client_tools": [],
            "model_speaks_first": false,
            "enable_message_streaming": offer.enable_message_streaming
        });
        if offer.mode == RealtimeMode::Wingman {
            let mut models_url = self.upstream.clone();
            models_url.set_path("/backend-api/models");
            models_url.query_pairs_mut().append_pair(
                "history_and_training_disabled",
                if offer.history_and_training_disabled {
                    "true"
                } else {
                    "false"
                },
            );
            let response = self
                .http
                .get(models_url.as_str())
                .headers(headers.clone())
                .send()
                .await
                .map_err(transport_error)?;
            let catalog: ModelCatalog = serde_json::from_str(&read_response(response).await?)
                .map_err(|_| RealtimeError::InvalidModelCatalog)?;
            if catalog.default_model_slug.is_empty()
                || catalog.default_model_slug.len() > 128
                || !catalog
                    .default_model_slug
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
            {
                return Err(RealtimeError::InvalidModelCatalog);
            }
            session["model_slug"] = json!("");
            session["requested_default_model"] = json!("");
            session["model_slug_advanced"] = json!("");
            session["backend_model"] = json!(catalog.default_model_slug);
            session["chat_mode"] = json!("chat");
            session["backend_reasoning_effort"] = json!("instant");
        }
        let mut url = self.upstream.clone();
        url.set_path(offer.mode.path());
        url.set_query(Some("dcid=0"));
        if let Some(token) = status
            .pointer("/chatreq/token")
            .and_then(|v| v.as_str())
            .or_else(|| status.get("chatreq_token").and_then(|v| v.as_str()))
            .filter(|v| !v.is_empty())
        {
            session["chatreq_token"] = json!(token);
        }
        let form = multipart::Form::new()
            .text("sdp", offer.sdp)
            .text("session", session.to_string());
        let response = self
            .http
            .post(url.as_str())
            .headers(headers)
            .multipart(form)
            .send()
            .await
            .map_err(transport_error)?;
        let body = read_response(response).await?;
        validate_sdp(&body).map_err(|_| RealtimeError::InvalidAnswer)?;
        Ok(RealtimeAnswer {
            sdp: body,
            session_id,
        })
    }
    fn auth_headers(&self) -> Result<header::HeaderMap, RealtimeError> {
        let mut headers = header::HeaderMap::new();
        let mut bearer =
            header::HeaderValue::from_str(&format!("Bearer {}", self.auth.access_token))
                .map_err(|_| RealtimeError::Authentication)?;
        bearer.set_sensitive(true);
        headers.insert(header::AUTHORIZATION, bearer);
        headers.insert(
            header::ORIGIN,
            header::HeaderValue::from_static("https://chatgpt.com"),
        );
        headers.insert(
            header::REFERER,
            header::HeaderValue::from_static("https://chatgpt.com/"),
        );
        if let Some(account) = &self.auth.account_id {
            let mut account = header::HeaderValue::from_str(account)
                .map_err(|_| RealtimeError::Authentication)?;
            account.set_sensitive(true);
            headers.insert("ChatGPT-Account-Id", account);
        }
        headers.insert("originator", header::HeaderValue::from_static(ORIGINATOR));
        headers.insert(
            header::USER_AGENT,
            header::HeaderValue::from_str(&desktop_user_agent())
                .map_err(|_| RealtimeError::Configuration)?,
        );
        Ok(headers)
    }
}

async fn read_response(response: wreq::Response) -> Result<String, RealtimeError> {
    let status = response.status().as_u16();
    let html = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("text/html"));
    if response.headers().contains_key("cf-mitigated") {
        return Err(RealtimeError::Challenge);
    }
    if response
        .content_length()
        .is_some_and(|len| len > MAX_SDP_BYTES as u64)
    {
        return Err(RealtimeError::TooLarge);
    }
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(transport_error)?;
        if body.len() + chunk.len() > MAX_SDP_BYTES {
            return Err(RealtimeError::TooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    let body = String::from_utf8(body).map_err(|_| RealtimeError::InvalidAnswer)?;
    let prefix = body
        .trim_start()
        .chars()
        .take(32)
        .collect::<String>()
        .to_ascii_lowercase();
    if html || prefix.starts_with("<!doctype html") || prefix.starts_with("<html") {
        return Err(RealtimeError::Challenge);
    }
    if !(200..300).contains(&status) {
        return Err(match status {
            401 => RealtimeError::AuthRejected,
            429 => RealtimeError::RateLimited,
            _ => RealtimeError::Upstream { status },
        });
    }
    Ok(body)
}

fn transport_error(error: wreq::Error) -> RealtimeError {
    if error.is_timeout() {
        RealtimeError::Timeout
    } else {
        RealtimeError::Transport
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_options_and_sdp_fail_before_network() {
        let mut offer = RealtimeOffer::new("v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\n".into());
        assert!(offer.validate().is_ok());
        offer.voice = "injected\nheader".into();
        assert!(offer.validate().is_err());
        offer.voice = default_voice();
        offer.timezone_offset_minutes = 900;
        assert!(offer.validate().is_err());
        assert!(RealtimeOffer::new("not SDP".into()).validate().is_err());
        assert!(matches!(
            RealtimeOffer::new("a".repeat(MAX_SDP_BYTES + 1)).validate(),
            Err(RealtimeError::TooLarge)
        ));
    }
}
