//! Subscription Quicksilver signaling. Call identifiers and authorization stay server-side.
use serde::{Deserialize, Serialize};
use serde_json::json;
use wreq::header;

use super::client::{read_response, transport_error};
use super::{RealtimeClient, RealtimeError, RealtimeOffer};

const INSTRUCTIONS: &str = "You are a voice conversation test assistant. Converse naturally. No backend agent or tools are attached. Never claim to execute tasks, look things up, or return verified tool results. Explain that limitation when asked.";

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LiveOffer {
    pub sdp: String,
    pub voice: String,
    /// Required acknowledgement; Quicksilver has no verified no-history setting.
    pub acknowledge_upstream_privacy: bool,
    #[serde(default = "streaming")]
    pub enable_message_streaming: bool,
}

fn streaming() -> bool {
    true
}

impl LiveOffer {
    pub fn validate(&self) -> Result<(), RealtimeError> {
        let mut offer = RealtimeOffer::new(self.sdp.clone());
        offer.voice.clone_from(&self.voice);
        offer.validate()?;
        if self
            .sdp
            .lines()
            .any(|line| line.starts_with("m=application "))
        {
            return Err(RealtimeError::InvalidOffer(
                "GPT-Live uses server control; data channel offers are unsupported",
            ));
        }
        if !self.acknowledge_upstream_privacy {
            return Err(RealtimeError::InvalidOffer("GPT-Live requires acknowledgement of upstream privacy semantics; Save off is not supported"));
        }
        if ![
            "arbor", "breeze", "cove", "ember", "juniper", "maple", "sol", "spruce", "vale",
        ]
        .contains(&self.voice.as_str())
        {
            return Err(RealtimeError::InvalidOffer(
                "unsupported GPT-Live subscription voice",
            ));
        }
        Ok(())
    }
}

/// Server-held call. Deliberately has no Serialize or Debug implementation.
pub(super) struct LiveCall {
    pub sdp: String,
    pub(crate) upstream_id: String,
    pub(crate) headers: header::HeaderMap,
}

impl RealtimeClient {
    pub(crate) fn live_sideband_url(&self, id: &str) -> Result<String, RealtimeError> {
        let mut url = if self.upstream.as_str().trim_end_matches('/') == super::DEFAULT_UPSTREAM {
            reqwest::Url::parse("https://api.openai.com")
                .map_err(|_| RealtimeError::Configuration)?
        } else {
            self.upstream.clone()
        };
        let scheme = if url.scheme() == "https" { "wss" } else { "ws" };
        url.set_scheme(scheme)
            .map_err(|_| RealtimeError::Configuration)?;
        url.set_path(&format!("/v1/live/{id}"));
        Ok(url.to_string())
    }

    pub(super) async fn create_live_call(
        &self,
        offer: &LiveOffer,
    ) -> Result<LiveCall, RealtimeError> {
        offer.validate()?;
        if self.auth.account_id.as_ref().is_none_or(|v| v.is_empty()) {
            return Err(RealtimeError::Authentication);
        }
        let mut headers = self.auth_headers()?;
        headers.insert(
            "openai-alpha",
            header::HeaderValue::from_static("quicksilver=v2"),
        );
        for name in ["session-id", "thread-id", "x-session-id"] {
            headers.insert(
                name,
                header::HeaderValue::from_str(&uuid::Uuid::new_v4().to_string())
                    .map_err(|_| RealtimeError::Configuration)?,
            );
        }
        let mut url = self.upstream.clone();
        url.set_path("/backend-api/codex/realtime/calls");
        url.set_query(Some("intent=quicksilver&architecture=avas"));
        let response = self
            .http
            .post(url.as_str())
            .headers(headers.clone())
            .json(&json!({
                "sdp": offer.sdp,
                "session": {
                    "model": "gpt-live-1-codex",
                    "instructions": INSTRUCTIONS,
                    "audio": {"output": {"voice": offer.voice}},
                    "delegation": {"type": "client"}
                }
            }))
            .send()
            .await
            .map_err(transport_error)?;
        eprintln!(
            "GPT-Live subscription negotiation returned HTTP {}",
            response.status().as_u16()
        );
        let id = call_id(response.headers());
        let sdp = read_response(response).await?;
        let upstream_id = id?;
        RealtimeOffer::new(sdp.clone())
            .validate()
            .map_err(|_| RealtimeError::InvalidAnswer)?;
        if [
            &self.auth.access_token,
            self.auth.account_id.as_deref().unwrap_or(""),
        ]
        .iter()
        .any(|secret| !secret.is_empty() && sdp.contains(*secret))
        {
            return Err(RealtimeError::InvalidAnswer);
        }
        Ok(LiveCall {
            sdp,
            upstream_id,
            headers,
        })
    }
}

fn valid_id(id: &str) -> bool {
    (id.starts_with("rtc_")
        && id.len() > 4
        && id.len() <= 132
        && id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c)))
        || (id.len() == 36 && uuid::Uuid::parse_str(id).is_ok())
}

fn call_id(headers: &header::HeaderMap) -> Result<String, RealtimeError> {
    let id = headers
        .get("openai-session-id")
        .and_then(|v| v.to_str().ok());
    let location = headers.get(header::LOCATION).and_then(|v| v.to_str().ok());
    let mut from_location = None;
    if let Some(location) = location {
        if location.len() > 512 {
            return Err(RealtimeError::InvalidAnswer);
        }
        let base = reqwest::Url::parse("https://api.openai.com")
            .map_err(|_| RealtimeError::Configuration)?;
        let url = base
            .join(location)
            .map_err(|_| RealtimeError::InvalidAnswer)?;
        if url.scheme() != "https"
            || !matches!(url.host_str(), Some("api.openai.com" | "chatgpt.com"))
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.port().is_some()
        {
            return Err(RealtimeError::InvalidAnswer);
        }
        from_location = url
            .path_segments()
            .and_then(|mut parts| parts.rfind(|s| valid_id(s)))
            .map(str::to_owned);
    }
    let result = match (id, from_location) {
        (Some(id), Some(other)) if id != other => return Err(RealtimeError::InvalidAnswer),
        (_, Some(other)) => other,
        (Some(id), None) => id.to_owned(),
        _ => return Err(RealtimeError::InvalidAnswer),
    };
    if !valid_id(&result) {
        return Err(RealtimeError::InvalidAnswer);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn account_identity_is_required_without_fallback() {
        let client = RealtimeClient::new(
            crate::CodexAuth::from_bearer("not-an-api-fallback", None).unwrap(),
            super::super::RealtimeClientConfig {
                proxy: None,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(matches!(
            client
                .create_live_call(&LiveOffer {
                    sdp: "v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\n".into(),
                    voice: "cove".into(),
                    acknowledge_upstream_privacy: true,
                    enable_message_streaming: true
                })
                .await,
            Err(RealtimeError::Authentication)
        ));
    }

    #[test]
    fn upstream_identifiers_are_bounded_and_locations_are_trusted() {
        for location in [
            "https://evil.example/rtc_test",
            "//evil.example/rtc_test",
            "/v1/live/rtc_test?token=secret",
            "https://api.openai.com:444/v1/live/rtc_test",
        ] {
            let mut headers = header::HeaderMap::new();
            headers.insert(header::LOCATION, location.parse().unwrap());
            assert!(call_id(&headers).is_err());
        }
        let mut headers = header::HeaderMap::new();
        headers.insert(header::LOCATION, "/v1/live/rtc_test".parse().unwrap());
        assert_eq!(call_id(&headers).unwrap(), "rtc_test");
        headers.insert("openai-session-id", "rtc_other".parse().unwrap());
        assert!(call_id(&headers).is_err());
    }
}
