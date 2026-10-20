//! Realtime signaling against a mock upstream (an in-process axum server that
//! imitates ChatGPT's voice endpoints). No live voice service is contacted.
#![cfg(all(feature = "server", feature = "realtime"))]

use axum::{
    extract::{Multipart, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use codex_voice::{
    realtime::{
        RealtimeClient, RealtimeClientConfig, RealtimeError, RealtimeMode, RealtimeOffer,
        MAX_SDP_BYTES,
    },
    CodexAuth,
};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

const SDP: &str = "v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\n";

#[derive(Clone)]
struct Mock {
    captured: Arc<Mutex<Vec<(String, Value)>>>,
    model_requests: Arc<Mutex<Vec<String>>>,
    models_body: String,
    status: StatusCode,
    content_type: &'static str,
    body: String,
}

async fn setup(
    status: StatusCode,
    content_type: &'static str,
    body: String,
) -> (RealtimeClient, Mock, tokio::task::JoinHandle<()>) {
    setup_with_models(
        status,
        content_type,
        body,
        json!({"default_model_slug": "current-default-model"}).to_string(),
    )
    .await
}

async fn setup_with_models(
    status: StatusCode,
    content_type: &'static str,
    body: String,
    models_body: String,
) -> (RealtimeClient, Mock, tokio::task::JoinHandle<()>) {
    let state = Mock {
        captured: Default::default(),
        model_requests: Default::default(),
        models_body,
        status,
        content_type,
        body,
    };
    let app = Router::new()
        .route(
            "/realtime/status",
            post(|headers: HeaderMap| async move {
                assert_eq!(headers["authorization"], "Bearer test-token");
                Json(json!({"chatreq_token": "private-status-ticket"}))
            }),
        )
        .route("/backend-api/models", get(models))
        .route("/realtime/{mode}", post(negotiate))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let auth = CodexAuth::from_bearer("test-token", Some("test-account".into())).unwrap();
    let client = RealtimeClient::new(
        auth,
        RealtimeClientConfig {
            upstream_base: base,
            proxy: None,
            ..Default::default()
        },
    )
    .unwrap();
    (client, state, task)
}

async fn models(State(state): State<Mock>, uri: axum::http::Uri, headers: HeaderMap) -> String {
    assert_eq!(headers["authorization"], "Bearer test-token");
    assert_eq!(headers["chatgpt-account-id"], "test-account");
    assert_eq!(headers["origin"], "https://chatgpt.com");
    state.model_requests.lock().unwrap().push(uri.to_string());
    state.models_body
}

async fn negotiate(
    State(state): State<Mock>,
    uri: axum::http::Uri,
    headers: HeaderMap,
    mut form: Multipart,
) -> (StatusCode, [(&'static str, &'static str); 1], String) {
    assert_eq!(headers["authorization"], "Bearer test-token");
    assert_eq!(headers["chatgpt-account-id"], "test-account");
    assert_eq!(headers["origin"], "https://chatgpt.com");
    let mut session = None;
    while let Some(field) = form.next_field().await.unwrap() {
        let name = field.name().unwrap().to_owned();
        let text = field.text().await.unwrap();
        match name.as_str() {
            "sdp" => assert_eq!(text, SDP),
            "session" => session = Some(serde_json::from_str::<Value>(&text).unwrap()),
            _ => panic!("unexpected multipart field"),
        }
    }
    state
        .captured
        .lock()
        .unwrap()
        .push((uri.to_string(), session.unwrap()));
    (
        state.status,
        [("content-type", state.content_type)],
        state.body,
    )
}

#[tokio::test]
async fn negotiation_uses_fixed_routes_status_ticket_and_whitelisted_session() {
    let (client, state, task) = setup(StatusCode::CREATED, "application/sdp", SDP.into()).await;
    let mut ids = Vec::new();
    for (mode, path, streaming) in [
        (RealtimeMode::Standard, "/realtime/vps?dcid=0", true),
        (RealtimeMode::Advanced, "/realtime/vp?dcid=0", false),
        (RealtimeMode::Wingman, "/realtime/wm?dcid=0", true),
    ] {
        let mut offer = RealtimeOffer::new(SDP.into());
        offer.mode = mode;
        offer.enable_message_streaming = streaming;
        offer.language = Some("uk".into());
        offer.timezone = "Europe/Kyiv".into();
        offer.timezone_offset_minutes = -180;
        let answer = client.create_session(offer).await.unwrap();
        assert_eq!(answer.sdp, SDP);
        let captures = state.captured.lock().unwrap();
        let (actual_path, session) = captures.last().unwrap();
        assert_eq!(actual_path, path);
        assert_eq!(session["chatreq_token"], "private-status-ticket");
        assert_eq!(session["voice_session_id"], answer.session_id);
        assert_eq!(session["voice_status_request_id"], answer.session_id);
        assert_eq!(session["voice"], "cove");
        assert_eq!(session["language_code"], "uk");
        assert_eq!(session["timezone_offset_min"], -180);
        assert_eq!(
            session["conversation_mode"],
            json!({"kind": "primary_assistant"})
        );
        assert_eq!(session["history_and_training_disabled"], true);
        assert_eq!(session["enable_message_streaming"], streaming);
        if mode == RealtimeMode::Wingman {
            assert_eq!(session["backend_model"], "current-default-model");
            assert_eq!(session["model_slug"], "");
            assert_eq!(session["requested_default_model"], "");
            assert_eq!(session["model_slug_advanced"], "");
            assert_eq!(session["chat_mode"], "chat");
            assert_eq!(session["backend_reasoning_effort"], "instant");
            assert_eq!(
                *state.model_requests.lock().unwrap(),
                vec!["/backend-api/models?history_and_training_disabled=true"]
            );
        } else {
            assert_eq!(session["backend_model"], "gpt-4o");
            assert_eq!(session["model_slug"], "gpt-4o");
            assert_eq!(session["requested_default_model"], "gpt-4o");
            assert!(session.get("model_slug_advanced").is_none());
            assert!(session.get("chat_mode").is_none());
            assert!(session.get("backend_reasoning_effort").is_none());
            assert!(state.model_requests.lock().unwrap().is_empty());
        }
        assert!(session.get("access_token").is_none());
        assert!(!serde_json::to_string(&answer)
            .unwrap()
            .contains("private-status-ticket"));
        ids.push(answer.session_id);
    }
    assert!(ids.windows(2).all(|w| w[0] != w[1]));
    client
        .create_session(RealtimeOffer::new(SDP.into()))
        .await
        .unwrap();
    let captures = state.captured.lock().unwrap();
    assert_eq!(captures.last().unwrap().1["language_code"], Value::Null);
    task.abort();
}

#[tokio::test]
async fn wingman_rejects_missing_or_malformed_default_models_before_negotiating() {
    for body in [
        "not JSON".into(),
        json!({}).to_string(),
        json!({"default_model_slug": null}).to_string(),
        json!({"default_model_slug": ""}).to_string(),
        json!({"default_model_slug": "private-token/invalid"}).to_string(),
        json!({"default_model_slug": "x".repeat(129)}).to_string(),
    ] {
        let (client, state, task) =
            setup_with_models(StatusCode::CREATED, "application/sdp", SDP.into(), body).await;
        let mut offer = RealtimeOffer::new(SDP.into());
        offer.mode = RealtimeMode::Wingman;
        let error = client.create_session(offer).await.unwrap_err();
        assert!(!error.to_string().contains("private-token"));
        assert!(state.captured.lock().unwrap().is_empty());
        task.abort();
    }
}

#[test]
fn offer_defaults_to_streaming_and_auto_language() {
    let offer: RealtimeOffer = serde_json::from_value(json!({"sdp": SDP})).unwrap();
    assert!(offer.enable_message_streaming);
    assert_eq!(offer.language, None);
    let offer: RealtimeOffer = serde_json::from_value(json!({
        "sdp": SDP, "language": null, "enable_message_streaming": false
    }))
    .unwrap();
    assert!(!offer.enable_message_streaming);
    assert_eq!(offer.language, None);
    let offer: RealtimeOffer =
        serde_json::from_value(json!({"sdp": SDP, "language": "uk"})).unwrap();
    assert_eq!(offer.language.as_deref(), Some("uk"));
}

#[tokio::test]
async fn history_choice_is_preserved_in_upstream_requests() {
    let (client, state, task) = setup(StatusCode::CREATED, "application/sdp", SDP.into()).await;
    for (mode, history) in [
        ("standard", None),
        ("advanced", Some(true)),
        ("wingman", Some(false)),
        ("standard", Some(false)),
        ("advanced", Some(false)),
    ] {
        let mut input = json!({"sdp": SDP, "mode": mode});
        if let Some(disabled) = history {
            input["history_and_training_disabled"] = json!(disabled);
        }
        let offer = serde_json::from_value(input).unwrap();
        client.create_session(offer).await.unwrap();
        let captures = state.captured.lock().unwrap();
        assert_eq!(
            captures.last().unwrap().1["history_and_training_disabled"],
            history.unwrap_or(true)
        );
        if mode == "wingman" {
            assert_eq!(
                *state.model_requests.lock().unwrap(),
                vec!["/backend-api/models?history_and_training_disabled=false"]
            );
        }
    }
    task.abort();
}

#[test]
fn history_defaults_are_private_and_require_a_boolean() {
    for offer in [
        RealtimeOffer::new(SDP.into()),
        serde_json::from_value(json!({"sdp": SDP})).unwrap(),
    ] {
        assert_eq!(
            serde_json::to_value(offer).unwrap()["history_and_training_disabled"],
            true
        );
    }
    for invalid in [json!(null), json!("false"), json!(0), json!({}), json!([])] {
        assert!(serde_json::from_value::<RealtimeOffer>(json!({
            "sdp": SDP, "history_and_training_disabled": invalid
        }))
        .is_err());
    }
}

#[tokio::test]
async fn upstream_errors_are_classified_without_echoing_private_bodies() {
    for (status, content_type, body, expected) in [
        (
            StatusCode::FORBIDDEN,
            "text/html",
            "<html>secret challenge</html>".into(),
            "challenge",
        ),
        (
            StatusCode::UNAUTHORIZED,
            "application/json",
            "{\"error\":{\"message\":\"private-token-value\"}}".into(),
            "auth",
        ),
        (
            StatusCode::TOO_MANY_REQUESTS,
            "application/json",
            "{}".into(),
            "limit",
        ),
        (
            StatusCode::OK,
            "text/plain",
            "x".repeat(MAX_SDP_BYTES + 1),
            "size",
        ),
        (
            StatusCode::OK,
            "application/json",
            "{\"sdp\":\"invalid\"}".into(),
            "invalid",
        ),
    ] {
        let (client, _, task) = setup(status, content_type, body).await;
        let error = client
            .create_session(RealtimeOffer::new(SDP.into()))
            .await
            .unwrap_err();
        assert!(!error.to_string().contains("private-token-value"));
        assert!(matches!(
            (expected, error),
            ("challenge", RealtimeError::Challenge)
                | ("auth", RealtimeError::AuthRejected)
                | ("limit", RealtimeError::RateLimited)
                | ("size", RealtimeError::TooLarge)
                | ("invalid", RealtimeError::InvalidAnswer)
        ));
        task.abort();
    }
}

#[test]
fn credentials_are_redacted_in_debug() {
    let auth = CodexAuth::from_bearer("private-token", Some("private-account".into())).unwrap();
    let output = format!("{auth:?}");
    assert!(!output.contains("private-token"));
    assert!(!output.contains("private-account"));
}

#[tokio::test]
async fn gpt_live_cannot_accidentally_use_the_legacy_signaling_contract() {
    let (client, state, task) = setup(StatusCode::CREATED, "application/sdp", SDP.into()).await;
    let offer = serde_json::from_value(json!({"sdp":SDP,"mode":"gpt_live"})).unwrap();
    let error = client.create_session(offer).await.unwrap_err();
    assert!(error.to_string().contains("/v1/realtime/live WebSocket"));
    assert!(state.captured.lock().unwrap().is_empty());
    task.abort();
}
