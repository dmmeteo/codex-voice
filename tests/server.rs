//! HTTP routes, local authorization, and error bodies.
//! The upstream ChatGPT endpoint is replaced by an in-process mock server;
//! these tests never contact a live transcription service.
#![cfg(feature = "server")]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::{to_bytes, Body};
use axum::extract::{Multipart, State};
use axum::http::{HeaderMap, Request, StatusCode};
use axum::routing::post;
use axum::{Json, Router};
use codex_voice::audio::SilkConfig;
use codex_voice::server::{router, ServerConfig, UpstreamConfig};
use codex_voice::AuthSource;
use serde_json::{json, Value};
use tower::ServiceExt;

const HOST: &str = "127.0.0.1:2600";
const KEY: &str = "local-key";
const WAV: &[u8] = b"RIFF\x24\x00\x00\x00WAVEfmt fixture-audio";
const BOUNDARY: &str = "codex-voice-test";

#[derive(Clone, Default)]
struct Upstream {
    uploads: Arc<Mutex<Vec<Value>>>,
}

/// Mock of the upstream transcribe endpoint.
async fn mock_upstream(transcribe_status: StatusCode) -> (String, Upstream) {
    let upstream = Upstream::default();
    let app =
        Router::new()
            .route(
                "/backend-api/transcribe",
                post(
                    move |State(upstream): State<Upstream>,
                          headers: HeaderMap,
                          mut form: Multipart| async move {
                        let mut seen = json!({
                            "authorization": headers["authorization"].to_str().unwrap(),
                            "originator": headers["originator"].to_str().unwrap(),
                        });
                        while let Some(field) = form.next_field().await.unwrap() {
                            let name = field.name().unwrap().to_owned();
                            if name == "file" {
                                seen["filename"] = json!(field.file_name());
                                seen["content_type"] = json!(field.content_type());
                                seen["bytes"] = json!(field.bytes().await.unwrap().len());
                            } else {
                                seen[name] = json!(field.text().await.unwrap());
                            }
                        }
                        upstream.uploads.lock().unwrap().push(seen);
                        (
                            transcribe_status,
                            Json(json!({"text": "fixture transcript"})),
                        )
                    },
                ),
            )
            .with_state(upstream.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (base, upstream)
}

fn config(base: &str, api_key: Option<&str>) -> ServerConfig {
    ServerConfig {
        api_key: api_key.map(str::to_string),
        auth: AuthSource::Bearer {
            token: "fixture-codex-token".into(),
            account_id: Some("fixture-account".into()),
        },
        upstream: UpstreamConfig {
            transcribe_endpoint: format!("{base}/backend-api/transcribe"),
            proxy: None,
            request_timeout: Duration::from_secs(5),
            connect_timeout: Duration::from_secs(5),
        },
        concurrency: 1,
        max_upload_bytes: 1024,
        silk: SilkConfig {
            decode: false,
            ..SilkConfig::default()
        },
        default_language: None,
    }
}

fn app(config: ServerConfig) -> Router {
    router(config)
}

fn multipart(parts: &[(&str, Option<&str>, &[u8])]) -> Vec<u8> {
    let mut body = Vec::new();
    for (name, filename, bytes) in parts {
        body.extend_from_slice(format!("--{BOUNDARY}\r\n").as_bytes());
        match filename {
            Some(filename) => body.extend_from_slice(
                format!("Content-Disposition: form-data; name=\"{name}\"; filename=\"{filename}\"\r\nContent-Type: application/octet-stream\r\n\r\n").as_bytes(),
            ),
            None => body.extend_from_slice(
                format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n").as_bytes(),
            ),
        }
        body.extend_from_slice(bytes);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{BOUNDARY}--\r\n").as_bytes());
    body
}

fn transcription_request(key: Option<&str>, body: Vec<u8>) -> Request<Body> {
    let mut request = Request::post("/v1/audio/transcriptions")
        .header("host", HOST)
        .header(
            "content-type",
            format!("multipart/form-data; boundary={BOUNDARY}"),
        );
    if let Some(key) = key {
        request = request.header("authorization", format!("Bearer {key}"));
    }
    request.body(Body::from(body)).unwrap()
}

async fn send(app: &Router, request: Request<Body>) -> (StatusCode, HeaderMap, Vec<u8>) {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let body = to_bytes(response.into_body(), 1 << 20).await.unwrap();
    (status, headers, body.to_vec())
}

fn error_body(body: &[u8]) -> Value {
    let value: Value = serde_json::from_slice(body).unwrap();
    assert!(value["error"]["message"].is_string(), "{value}");
    value["error"].clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn transcription_uploads_sniffed_audio_and_formats_each_response() {
    let (base, upstream) = mock_upstream(StatusCode::OK).await;
    let app = app(config(&base, Some(KEY)));
    for (format, content_type, expected) in [
        (
            "json",
            "application/json",
            r#"{"text":"fixture transcript"}"#,
        ),
        ("text", "text/plain; charset=utf-8", "fixture transcript"),
        (
            "verbose_json",
            "application/json",
            r#"{"duration":null,"language":"uk","segments":[],"task":"transcribe","text":"fixture transcript"}"#,
        ),
    ] {
        let body = multipart(&[
            ("model", None, b"whisper-1"),
            ("file", Some("audio"), WAV),
            ("language", None, b"uk"),
            ("response_format", None, format.as_bytes()),
        ]);
        let (status, headers, body) = send(&app, transcription_request(Some(KEY), body)).await;
        assert_eq!(status, StatusCode::OK, "{format}");
        assert_eq!(headers["content-type"], content_type);
        let body = String::from_utf8(body).unwrap();
        if format == "verbose_json" {
            assert_eq!(
                serde_json::from_str::<Value>(&body).unwrap(),
                serde_json::from_str::<Value>(expected).unwrap()
            );
        } else {
            assert_eq!(body, expected);
        }
    }
    let uploads = upstream.uploads.lock().unwrap();
    assert_eq!(uploads.len(), 3);
    assert_eq!(
        uploads[0],
        json!({
            "authorization": "Bearer fixture-codex-token",
            "originator": "Codex Desktop",
            "filename": "audio.wav",
            "content_type": "audio/wav",
            "bytes": WAV.len(),
            "language": "uk",
        })
    );
}

/// API key, multipart body, expected status, error type, error message.
type ErrorCase = (
    Option<&'static str>,
    Vec<u8>,
    StatusCode,
    &'static str,
    &'static str,
);

#[tokio::test(flavor = "multi_thread")]
async fn transcription_errors_keep_their_status_and_openai_error_shape() {
    let (base, _) = mock_upstream(StatusCode::OK).await;
    let app = app(config(&base, Some(KEY)));
    let cases: Vec<ErrorCase> = vec![
        (
            None,
            multipart(&[("file", Some("a.wav"), WAV)]),
            StatusCode::UNAUTHORIZED,
            "authentication_error",
            "missing or invalid bearer token",
        ),
        (
            Some("wrong"),
            multipart(&[("file", Some("a.wav"), WAV)]),
            StatusCode::UNAUTHORIZED,
            "authentication_error",
            "missing or invalid bearer token",
        ),
        (
            Some(KEY),
            multipart(&[("language", None, b"en")]),
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            "missing multipart file field",
        ),
        (
            Some(KEY),
            multipart(&[("file", Some("a.wav"), b"")]),
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            "file is empty",
        ),
        (
            Some(KEY),
            multipart(&[
                ("file", Some("a.wav"), WAV),
                ("response_format", None, b"srt"),
            ]),
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            "response_format srt/vtt is not supported because timestamps are unavailable",
        ),
        (
            Some(KEY),
            multipart(&[
                ("file", Some("a.wav"), WAV),
                ("response_format", None, b"xml"),
            ]),
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            "unsupported response_format: xml",
        ),
        (
            Some(KEY),
            multipart(&[("file", Some("a.wav"), &[7u8; 2048])]),
            StatusCode::PAYLOAD_TOO_LARGE,
            "invalid_request_error",
            "multipart body exceeds the configured upload limit",
        ),
    ];
    for (key, body, status, error_type, message) in cases {
        let (actual, _, body) = send(&app, transcription_request(key, body)).await;
        assert_eq!(
            actual,
            status,
            "{message}: {}",
            String::from_utf8_lossy(&body)
        );
        let error = error_body(&body);
        assert_eq!(error["type"], error_type);
        assert_eq!(error["message"], message);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn upstream_transcription_failure_is_a_bad_gateway() {
    let (base, _) = mock_upstream(StatusCode::INTERNAL_SERVER_ERROR).await;
    let app = app(config(&base, None));
    let (status, _, body) = send(
        &app,
        transcription_request(None, multipart(&[("file", Some("a.wav"), WAV)])),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    let error = error_body(&body);
    assert_eq!(error["type"], "upstream_error");
    assert!(error["message"]
        .as_str()
        .unwrap()
        .starts_with("upstream transcribe failed: transcribe request failed with HTTP 500"));
}

#[tokio::test(flavor = "multi_thread")]
async fn cross_site_browsers_cannot_use_the_transcription_route() {
    let (base, upstream) = mock_upstream(StatusCode::OK).await;
    let app = app(config(&base, None));
    for (origin, fetch_site) in [
        (Some("https://attacker.example"), None),
        (None, Some("cross-site")),
        (Some("http://127.0.0.1:2600"), Some("same-site")),
    ] {
        let mut request = transcription_request(None, multipart(&[("file", Some("a.wav"), WAV)]));
        if let Some(origin) = origin {
            request
                .headers_mut()
                .insert("origin", origin.parse().unwrap());
        }
        if let Some(site) = fetch_site {
            request
                .headers_mut()
                .insert("sec-fetch-site", site.parse().unwrap());
        }
        let (status, _, body) = send(&app, request).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(error_body(&body)["type"], "origin_error");
    }
    assert!(upstream.uploads.lock().unwrap().is_empty());
    let mut same_origin = transcription_request(None, multipart(&[("file", Some("a.wav"), WAV)]));
    same_origin
        .headers_mut()
        .insert("origin", "http://127.0.0.1:2600".parse().unwrap());
    same_origin
        .headers_mut()
        .insert("sec-fetch-site", "same-origin".parse().unwrap());
    assert_eq!(send(&app, same_origin).await.0, StatusCode::OK);
}
