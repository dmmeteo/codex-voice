//! Controlled subscription and sideband tests. No live upstream or paid API calls.
#![cfg(all(feature = "server", feature = "realtime"))]
use axum::{
    extract::{
        ws::{Message, WebSocketUpgrade},
        State,
    },
    http::{HeaderMap, StatusCode, Uri},
    routing::{get, post},
    Json, Router,
};
use codex_voice::{
    audio::SilkConfig,
    server::{router, ServerConfig, UpstreamConfig},
    AuthSource,
};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message as ClientMessage},
};

const SDP: &str = "v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\n";
#[derive(Clone, Default)]
struct Mock {
    requests: Arc<Mutex<Vec<Value>>>,
    commands: Arc<Mutex<Vec<Value>>>,
    delay: bool,
    reject: bool,
}
async fn upstream(
    State(mock): State<Mock>,
    headers: HeaderMap,
    uri: Uri,
    Json(payload): Json<Value>,
) -> (StatusCode, HeaderMap, String) {
    assert_eq!(uri.query(), Some("intent=quicksilver&architecture=avas"));
    assert_eq!(headers["authorization"], "Bearer upstream-secret");
    assert_eq!(headers["chatgpt-account-id"], "private-account");
    assert_eq!(headers["openai-alpha"], "quicksilver=v2");
    for name in ["session-id", "thread-id", "x-session-id"] {
        uuid::Uuid::parse_str(headers[name].to_str().unwrap()).unwrap();
    }
    mock.requests.lock().unwrap().push(payload);
    if mock.reject {
        return (
            StatusCode::FORBIDDEN,
            HeaderMap::new(),
            "private upstream rejection with upstream-secret and private-account".into(),
        );
    }
    if mock.delay {
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
    let mut headers = HeaderMap::new();
    headers.insert("openai-session-id", "rtc_fixture".parse().unwrap());
    (StatusCode::CREATED, headers, SDP.into())
}
async fn sideband(
    State(mock): State<Mock>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> axum::response::Response {
    assert_eq!(headers["authorization"], "Bearer upstream-secret");
    assert_eq!(headers["chatgpt-account-id"], "private-account");
    assert_eq!(headers["openai-alpha"], "quicksilver=v2");
    ws.on_upgrade(move |mut socket| async move {
        for event in [
            json!({"type":"session.started","session":{"id":"rtc_fixture","private":"upstream-secret"}}),
            json!({"type":"turn.done","turn":{"role":"user","transcript":"Hello"}}),
            json!({"type":"session.output_transcript.delta","delta":"Hi there"}),
            json!({"type":"delegation.created","item":{"type":"delegation","id":"delegation_one","target":"client","content":[{"type":"input_text","text":"Search the web"}]}}),
        ] { socket.send(Message::Text(event.to_string().into())).await.unwrap(); }
        while let Some(Ok(message)) = socket.recv().await {
            if let Message::Text(text) = message {
                let event: Value = serde_json::from_str(&text).unwrap();
                mock.commands.lock().unwrap().push(event);
            }
        }
    })
}
async fn fixture(
    delay: bool,
    reject: bool,
) -> (
    String,
    String,
    Mock,
    tokio::task::JoinHandle<()>,
    tokio::task::JoinHandle<()>,
) {
    let mock = Mock {
        delay,
        reject,
        ..Default::default()
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new()
        .route(
            "/transcribe",
            post(|| async { Json(json!({"text":"existing file transcription"})) }),
        )
        .route("/backend-api/codex/realtime/calls", post(upstream))
        .route("/v1/live/rtc_fixture", get(sideband))
        .with_state(mock.clone());
    let upstream_task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = router(
        ServerConfig {
            api_key: Some("local-key".into()),
            auth: AuthSource::Bearer {
                token: "upstream-secret".into(),
                account_id: Some("private-account".into()),
            },
            upstream: UpstreamConfig {
                transcribe_endpoint: format!("{base}/transcribe"),
                realtime_base: base.clone(),
                proxy: None,
                request_timeout: Duration::from_secs(5),
                connect_timeout: Duration::from_secs(2),
            },
            concurrency: 1,
            max_upload_bytes: 10 * 1024 * 1024,
            silk: SilkConfig::default(),
            default_language: None,
            allowed_hosts: vec![],
        },
        addr,
    );
    let server_task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (
        format!("ws://{addr}/v1/realtime/live"),
        base,
        mock,
        upstream_task,
        server_task,
    )
}
fn start(key: &str, privacy: bool) -> ClientMessage {
    ClientMessage::Text(json!({"api_key":key,"offer":{"sdp":SDP,"voice":"cove","acknowledge_upstream_privacy":privacy,"enable_message_streaming":true}}).to_string().into())
}

#[tokio::test]
async fn subscription_request_projection_delegation_and_stop_are_real_boundaries() {
    let (url, _, mock, upstream, server) = fixture(false, false).await;
    let (mut socket, _) = connect_async(&url).await.unwrap();
    socket.send(start("local-key", true)).await.unwrap();
    let mut seen = Vec::new();
    while seen.len() < 6 {
        let message = tokio::time::timeout(Duration::from_secs(5), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        if let ClientMessage::Text(text) = message {
            assert!(!text.contains("upstream-secret"));
            assert!(!text.contains("private-account"));
            assert!(!text.contains("rtc_fixture"));
            seen.push(serde_json::from_str::<Value>(&text).unwrap());
        }
    }
    assert_eq!(seen[0]["type"], "answer");
    assert_eq!(seen[0]["sdp"], SDP);
    uuid::Uuid::parse_str(seen[0]["session_id"].as_str().unwrap()).unwrap();
    assert_eq!(seen[1], json!({"type":"live_ready"}));
    assert_eq!(seen[2], json!({"type":"session.started"}));
    assert_eq!(
        seen[3],
        json!({"type":"live_transcript","role":"user","text":"Hello","append":false,"final":true})
    );
    assert_eq!(seen[4]["text"], "Hi there");
    assert_eq!(seen[5], json!({"type":"delegation_unavailable"}));
    {
        let requests = mock.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        let session = &requests[0]["session"];
        assert_eq!(requests[0]["sdp"], SDP);
        assert_eq!(session["model"], "gpt-live-1-codex");
        assert_eq!(session["audio"], json!({"output":{"voice":"cove"}}));
        assert_eq!(session["delegation"], json!({"type":"client"}));
        assert!(session["instructions"]
            .as_str()
            .unwrap()
            .contains("No backend agent"));
        assert_eq!(session.as_object().unwrap().len(), 4);
    }
    let transcription_url = url
        .replace("ws://", "http://")
        .replace("/v1/realtime/live", "/v1/audio/transcriptions");
    let response = reqwest::Client::new()
        .post(transcription_url)
        .bearer_auth("local-key")
        .timeout(Duration::from_secs(5))
        .multipart(
            reqwest::multipart::Form::new().part(
                "file",
                reqwest::multipart::Part::bytes(b"RIFF public fixture".to_vec())
                    .file_name("public.wav")
                    .mime_str("audio/wav")
                    .unwrap(),
            ),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.json::<Value>().await.unwrap(),
        json!({"text":"existing file transcription"})
    );
    let second = connect_async(&url).await.unwrap_err();
    assert!(
        matches!(second, tokio_tungstenite::tungstenite::Error::Http(response) if response.status() == StatusCode::TOO_MANY_REQUESTS)
    );
    socket
        .send(ClientMessage::Text("stop".into()))
        .await
        .unwrap();
    while let Some(message) = socket.next().await {
        if matches!(message.unwrap(), ClientMessage::Close(_)) {
            break;
        }
    }
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if mock
                .commands
                .lock()
                .unwrap()
                .iter()
                .any(|v| v["type"] == "session.close")
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let commands = mock.commands.lock().unwrap();
    assert_eq!(commands[0]["type"], "delegation.context.append");
    assert_eq!(commands[0]["delegation_item_id"], "delegation_one");
    assert!(commands[0]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("cannot execute"));
    upstream.abort();
    server.abort();
}

#[tokio::test]
async fn origin_key_and_privacy_rejections_never_negotiate() {
    let (url, _, mock, upstream, server) = fixture(false, false).await;
    let mut request = url.clone().into_client_request().unwrap();
    request
        .headers_mut()
        .insert("origin", "https://attacker.example".parse().unwrap());
    assert!(
        matches!(connect_async(request).await.unwrap_err(), tokio_tungstenite::tungstenite::Error::Http(response) if response.status() == StatusCode::FORBIDDEN)
    );
    let mut request = url.clone().into_client_request().unwrap();
    request
        .headers_mut()
        .insert("host", "rebind.invalid".parse().unwrap());
    assert!(
        matches!(connect_async(request).await.unwrap_err(), tokio_tungstenite::tungstenite::Error::Http(response) if response.status() == StatusCode::FORBIDDEN)
    );
    for (key, privacy, expected) in [
        ("wrong", true, "Authentication failed"),
        ("local-key", false, "acknowledgement"),
    ] {
        let (mut socket, _) = connect_async(&url).await.unwrap();
        socket.send(start(key, privacy)).await.unwrap();
        let event = socket.next().await.unwrap().unwrap().into_text().unwrap();
        assert!(event.contains(expected));
        while let Some(message) = socket.next().await {
            if matches!(message.unwrap(), ClientMessage::Close(_)) {
                break;
            }
        }
        socket.close(None).await.ok();
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(mock.requests.lock().unwrap().is_empty());
    upstream.abort();
    server.abort();
}

#[tokio::test]
async fn stop_during_signaling_cancels_and_releases_capacity() {
    let (url, _, mock, upstream, server) = fixture(true, false).await;
    let (mut socket, _) = connect_async(&url).await.unwrap();
    socket.send(start("local-key", true)).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while mock.requests.lock().unwrap().is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    socket.close(None).await.unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;
    let (mut next, _) = connect_async(&url).await.unwrap();
    next.send(start("bad", true)).await.unwrap();
    assert!(next
        .next()
        .await
        .unwrap()
        .unwrap()
        .into_text()
        .unwrap()
        .contains("Authentication failed"));
    assert!(mock.commands.lock().unwrap().is_empty());
    upstream.abort();
    server.abort();
}

#[tokio::test]
async fn upstream_rejection_is_exactly_classified_without_private_body_or_fallback() {
    let (url, _, mock, upstream, server) = fixture(false, true).await;
    let (mut socket, _) = connect_async(&url).await.unwrap();
    socket.send(start("local-key", true)).await.unwrap();
    let text = socket.next().await.unwrap().unwrap().into_text().unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&text).unwrap(),
        json!({"type":"error","error":{"message":"ChatGPT rejected the voice session (HTTP 403)"}})
    );
    assert_eq!(mock.requests.lock().unwrap().len(), 1);
    assert!(mock.commands.lock().unwrap().is_empty());
    upstream.abort();
    server.abort();
}
