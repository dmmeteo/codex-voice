//! A connection owns subscription signaling and the trusted sideband.
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::Response;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message as UpstreamMessage};

use super::{LiveOffer, RealtimeClient, RealtimeClientConfig};
use crate::server::{ApiError, AppState};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Start {
    api_key: String,
    offer: LiveOffer,
}

pub(crate) async fn connect(
    State(state): State<AppState>,
    ws: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    let slot = state
        .live_limiter
        .clone()
        .try_acquire_owned()
        .map_err(|_| {
            ApiError::new(
                axum::http::StatusCode::TOO_MANY_REQUESTS,
                "GPT-Live control capacity reached",
                "rate_limit_error",
            )
        })?;
    Ok(ws
        .max_message_size(super::MAX_SDP_BYTES * 2 + 4096)
        .max_frame_size(super::MAX_SDP_BYTES * 2 + 4096)
        .on_upgrade(move |mut socket| async move {
            let _slot = slot;
            let result = run(&state, &mut socket).await;
            if let Err(message) = result {
                let _ = send(
                    &mut socket,
                    json!({"type":"error", "error":{"message": message}}),
                )
                .await;
            }
            let _ = socket.close().await;
        }))
}

async fn send(socket: &mut WebSocket, value: Value) -> Result<(), String> {
    tokio::time::timeout(
        Duration::from_secs(10),
        socket.send(Message::Text(value.to_string().into())),
    )
    .await
    .map_err(|_| "GPT-Live client is not receiving events".to_owned())?
    .map_err(|_| "GPT-Live client disconnected".to_owned())
}

async fn run(state: &AppState, socket: &mut WebSocket) -> Result<(), String> {
    let start = tokio::time::timeout(Duration::from_secs(10), socket.recv())
        .await
        .map_err(|_| "GPT-Live authentication timed out")?;
    let Some(Ok(Message::Text(text))) = start else {
        return Err("GPT-Live requires a JSON start frame".into());
    };
    let start: Start = serde_json::from_str(&text).map_err(|_| "invalid GPT-Live start frame")?;
    let bearer = format!("Bearer {}", start.api_key);
    if !crate::auth::is_authorized(Some(&bearer), state.config.api_key.as_deref()) {
        return Err("Authentication failed (401). Enter this server's API key.".into());
    }
    start.offer.validate().map_err(|e| e.to_string())?;
    let permit = state
        .limiter
        .clone()
        .try_acquire_owned()
        .map_err(|_| "server is busy; try again shortly")?;
    let config = &state.config;
    let auth = config
        .auth
        .load()
        .map_err(|_| "cannot load Codex authentication; renew the server's Codex login")?;
    let client = RealtimeClient::new(
        auth,
        RealtimeClientConfig {
            upstream_base: config.upstream.realtime_base.clone(),
            proxy: config.upstream.proxy.clone(),
            timeout: config.upstream.request_timeout,
            connect_timeout: config.upstream.connect_timeout,
        },
    )
    .map_err(|e| e.to_string())?;
    let call = tokio::select! {
        result = client.create_live_call(&start.offer) => result.map_err(|e| e.to_string())?,
        _ = socket.recv() => return Ok(()),
    };
    drop(permit);
    send(
        socket,
        json!({"type":"answer", "sdp":call.sdp, "session_id":uuid::Uuid::new_v4().to_string()}),
    )
    .await?;
    let mut request = client
        .live_sideband_url(&call.upstream_id)
        .map_err(|e| e.to_string())?
        .into_client_request()
        .map_err(|_| "invalid GPT-Live sideband configuration")?;
    for (name, value) in &call.headers {
        request.headers_mut().insert(name.clone(), value.clone());
    }
    let connect = tokio_tungstenite::connect_async_with_config(
        request,
        Some(
            tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default()
                .max_message_size(Some(128 * 1024))
                .max_frame_size(Some(128 * 1024)),
        ),
        false,
    );
    let (mut upstream, _) = tokio::select! {
        result = tokio::time::timeout(config.upstream.connect_timeout, connect) => result.map_err(|_| "GPT-Live sideband connection timed out")?.map_err(|error| match error {
            tokio_tungstenite::tungstenite::Error::Http(response) => format!("GPT-Live sideband rejected connection (HTTP {})", response.status().as_u16()),
            _ => "GPT-Live sideband connection rejected or unavailable".into(),
        })?,
        _ = socket.recv() => return Ok(()),
    };
    let lifetime = tokio::time::sleep(Duration::from_secs(60 * 60));
    tokio::pin!(lifetime);
    let result = async {
        send(socket, json!({"type":"live_ready"})).await?;
        loop {
            tokio::select! {
                _ = &mut lifetime => return Err("GPT-Live reached the local one-hour session limit".into()),
                incoming = socket.recv() => match incoming {
                    Some(Ok(Message::Text(text))) if text.as_str() == "stop" => return Ok(()),
                    Some(Ok(Message::Ping(_))) | Some(Ok(Message::Pong(_))) => {},
                    None | Some(Ok(Message::Close(_))) => return Ok(()),
                    _ => return Err("unsupported GPT-Live control command".into()),
                },
                incoming = upstream.next() => {
                    let text = match incoming {
                        Some(Ok(UpstreamMessage::Text(text))) => text,
                        Some(Ok(UpstreamMessage::Ping(_))) | Some(Ok(UpstreamMessage::Pong(_))) => continue,
                        _ => return Err("GPT-Live sideband disconnected".into()),
                    };
                    let event: Value = serde_json::from_str(&text).map_err(|_| "GPT-Live returned a malformed event")?;
                    if let Some(id) = delegation_id(&event) {
                        let response = json!({"type":"delegation.context.append", "delegation_item_id":id, "channel":"speakable", "content":[{"type":"input_text", "text":"No backend agent or tools are connected. I cannot execute that task or verify external information."}]});
                        tokio::time::timeout(Duration::from_secs(10), upstream.send(UpstreamMessage::Text(response.to_string().into()))).await.map_err(|_| "GPT-Live delegation response timed out")?.map_err(|_| "GPT-Live delegation response failed")?;
                        send(socket, json!({"type":"delegation_unavailable"})).await?;
                    }
                    if let Some(event) = public_event(&event, start.offer.enable_message_streaming, &[&client.auth.access_token, client.auth.account_id.as_deref().unwrap_or(""), &call.upstream_id]) {
                        let ended = matches!(event["type"].as_str(), Some("session.closed" | "error"));
                        send(socket, event).await?;
                        if ended { return Ok(()); }
                    }
                }
            }
        }
    }.await;
    let _ = tokio::time::timeout(Duration::from_secs(2), async {
        upstream
            .send(UpstreamMessage::Text(
                json!({"type":"session.close"}).to_string().into(),
            ))
            .await?;
        upstream.close(None).await
    })
    .await;
    result
}

fn delegation_id(event: &Value) -> Option<&str> {
    let item = match event["type"].as_str()? {
        "delegation.created" => &event["item"],
        "session.delegation.created" => &event["delegation"],
        _ => return None,
    };
    if item["target"] != "client" || item["type"] != "delegation" {
        return None;
    }
    item["id"].as_str().filter(|id| {
        !id.is_empty()
            && id.len() <= 512
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
    })
}

fn public_event(event: &Value, streaming: bool, secrets: &[&str]) -> Option<Value> {
    let kind = event["type"].as_str()?;
    match kind {
        "session.started" => return Some(json!({"type":"session.started"})),
        "session.closed" => return Some(json!({"type":"session.closed"})),
        "error" => {
            return Some(
                json!({"type":"error", "error":{"message":"GPT-Live rejected a session command"}}),
            )
        }
        _ => {}
    }
    if !streaming {
        return None;
    }
    let (role, text, append, finality) = match kind {
        "session.input_transcript.delta" => ("user", event["delta"].as_str()?, true, false),
        "session.output_transcript.delta" => ("assistant", event["delta"].as_str()?, true, false),
        "input_transcript.added" => ("user", event["item"]["text"].as_str()?, true, false),
        "output_transcript.added" => ("assistant", event["item"]["text"].as_str()?, true, false),
        "turn.done" => (
            event["turn"]["role"].as_str()?,
            event["turn"]["transcript"].as_str()?,
            false,
            true,
        ),
        _ => return None,
    };
    if !["user", "assistant"].contains(&role)
        || text.len() > 16_000
        || secrets.iter().any(|s| !s.is_empty() && text.contains(*s))
    {
        return None;
    }
    Some(
        json!({"type":"live_transcript", "role":role, "text":text, "append":append, "final":finality}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn subscription_transcript_items_are_fragments_until_turn_finality() {
        assert_eq!(
            public_event(
                &json!({"type":"input_transcript.added","item":{"text":"Hello "}}),
                true,
                &[]
            ),
            Some(
                json!({"type":"live_transcript","role":"user","text":"Hello ","append":true,"final":false})
            )
        );
        assert_eq!(
            public_event(
                &json!({"type":"output_transcript.added","item":{"text":"there"}}),
                true,
                &[]
            ),
            Some(
                json!({"type":"live_transcript","role":"assistant","text":"there","append":true,"final":false})
            )
        );
    }

    #[test]
    fn sideband_projection_never_relays_sessions_capabilities_or_provider_errors() {
        assert_eq!(
            public_event(
                &json!({"type":"session.started", "session":{"id":"private", "token":"secret"}}),
                true,
                &[]
            ),
            Some(json!({"type":"session.started"}))
        );
        assert!(!public_event(
            &json!({"type":"error", "error":{"message":"secret"}}),
            true,
            &[]
        )
        .unwrap()
        .to_string()
        .contains("secret"));
        assert!(public_event(
            &json!({"type":"session.output_transcript.delta", "delta":"secret"}),
            true,
            &["secret"]
        )
        .is_none());
        assert!(public_event(
            &json!({"type":"session.output_transcript.delta", "delta":"hi"}),
            false,
            &[]
        )
        .is_none());
        assert_eq!(
            public_event(
                &json!({"type":"turn.done", "turn":{"role":"assistant", "transcript":"Hello"}}),
                true,
                &[]
            ),
            Some(
                json!({"type":"live_transcript","role":"assistant","text":"Hello","append":false,"final":true})
            )
        );
    }
}
