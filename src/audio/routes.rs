//! `POST /v1/audio/transcriptions`, shaped like the OpenAI Whisper route.
use std::io::Write;
use std::path::Path;

use axum::body::Body;
use axum::extract::multipart::{Field, MultipartError};
use axum::extract::{Multipart, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use tempfile::NamedTempFile;

use super::{decode_silk, format_for_content_type, infer_content_type, is_silk_path, AudioFormat};
use super::{DecodedSilk, TranscribeOptions, Transcription, TranscriptionClient};
use crate::server::{ApiError, AppState};

pub(crate) async fn create_transcription(
    State(state): State<AppState>,
    headers: HeaderMap,
    multipart: Multipart,
) -> Result<Response, ApiError> {
    state.authorize(&headers)?;
    let request =
        parse_transcription_multipart(multipart, state.config.default_language.clone()).await?;
    let response_format = ResponseFormat::parse(request.response_format.as_deref())?;
    let permit = state
        .limiter
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| ApiError::internal("concurrency limiter is closed"))?;
    let language = request.language.clone();
    let transcription = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        transcribe_upload(&state, request)
    })
    .await
    .map_err(|error| ApiError::internal(format!("transcribe worker failed: {error}")))??;
    Ok(format_transcription_response(
        &transcription.text,
        response_format,
        language.as_deref(),
    ))
}

struct TranscriptionRequest {
    file: NamedTempFile,
    filename: String,
    content_type: String,
    language: Option<String>,
    response_format: Option<String>,
}

struct TranscriptionUpload {
    file: NamedTempFile,
    filename: String,
    content_type: String,
}

async fn parse_transcription_multipart(
    mut multipart: Multipart,
    default_language: Option<String>,
) -> Result<TranscriptionRequest, ApiError> {
    let mut file = None;
    let mut language = default_language;
    let mut response_format = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|error| multipart_error(error, "invalid multipart body"))?
    {
        match field.name().unwrap_or("") {
            "file" => file = Some(read_upload(field).await?),
            "language" => language = Some(read_text_field(field).await?),
            "response_format" => response_format = Some(read_text_field(field).await?),
            // OpenAI clients also send model, prompt, temperature and
            // timestamp_granularities; upstream has no equivalent, so drain them.
            _ => {
                let _ = field.bytes().await;
            }
        }
    }
    let Some(upload) = file else {
        return Err(ApiError::bad_request("missing multipart file field"));
    };
    Ok(TranscriptionRequest {
        file: upload.file,
        filename: upload.filename,
        content_type: upload.content_type,
        language,
        response_format,
    })
}

async fn read_upload(field: Field<'_>) -> Result<TranscriptionUpload, ApiError> {
    let raw_filename = sanitize_filename(field.file_name().unwrap_or("audio"));
    let raw_content_type = field
        .content_type()
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| infer_content_type(&raw_filename).to_string());
    let bytes = field
        .bytes()
        .await
        .map_err(|error| multipart_error(error, "failed to read file"))?;
    if bytes.is_empty() {
        return Err(ApiError::bad_request("file is empty"));
    }
    let (filename, content_type) =
        normalize_upload_metadata(&raw_filename, raw_content_type, &bytes);
    let mut temp = tempfile::Builder::new()
        .prefix("codex-voice-upload-")
        .suffix(&upload_suffix(&filename, &content_type))
        .tempfile()
        .map_err(|error| ApiError::internal(format!("failed to create temp file: {error}")))?;
    temp.write_all(&bytes)
        .map_err(|error| ApiError::internal(format!("failed to write temp file: {error}")))?;
    Ok(TranscriptionUpload {
        file: temp,
        filename,
        content_type,
    })
}

async fn read_text_field(field: Field<'_>) -> Result<String, ApiError> {
    field
        .text()
        .await
        .map(|text| text.trim().to_string())
        .map_err(|error| multipart_error(error, "failed to read text field"))
}

fn multipart_error(error: MultipartError, context: &str) -> ApiError {
    if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
        ApiError::payload_too_large("multipart body exceeds the configured upload limit")
    } else {
        ApiError::bad_request(format!("{context}: {error}"))
    }
}

fn transcribe_upload(
    state: &AppState,
    request: TranscriptionRequest,
) -> Result<Transcription, ApiError> {
    let config = &state.config;
    let decoded = decode_silk(request.file.path(), &config.silk)
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    let (path, filename, content_type) = match &decoded {
        Some(decoded) => (
            decoded.path(),
            decoded.filename.clone(),
            DecodedSilk::CONTENT_TYPE.to_string(),
        ),
        None => (request.file.path(), request.filename, request.content_type),
    };
    let auth = config
        .auth
        .load()
        .map_err(|error| ApiError::internal(format!("failed to load Codex auth: {error}")))?;
    let client = TranscriptionClient::builder(auth)
        .endpoint(config.upstream.transcribe_endpoint.clone())
        .proxy(config.upstream.proxy.clone())
        .timeout(Some(config.upstream.request_timeout))
        .connect_timeout(Some(config.upstream.connect_timeout))
        .build()
        .map_err(|error| ApiError::internal(format!("failed to build ASR client: {error}")))?;
    client
        .transcribe_file(
            path,
            TranscribeOptions {
                language: request.language,
                content_type: Some(content_type),
                filename: Some(filename),
            },
        )
        .map_err(|error| ApiError::bad_gateway(format!("upstream transcribe failed: {error}")))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResponseFormat {
    Json,
    Text,
    VerboseJson,
}

impl ResponseFormat {
    fn parse(value: Option<&str>) -> Result<Self, ApiError> {
        match value.unwrap_or("json").trim() {
            "" | "json" => Ok(Self::Json),
            "text" => Ok(Self::Text),
            "verbose_json" => Ok(Self::VerboseJson),
            "srt" | "vtt" => Err(ApiError::bad_request(
                "response_format srt/vtt is not supported because timestamps are unavailable",
            )),
            other => Err(ApiError::bad_request(format!(
                "unsupported response_format: {other}"
            ))),
        }
    }
}

fn format_transcription_response(
    text: &str,
    format: ResponseFormat,
    language: Option<&str>,
) -> Response {
    match format {
        ResponseFormat::Json => Json(serde_json::json!({ "text": text })).into_response(),
        ResponseFormat::Text => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
            Body::from(text.to_string()),
        )
            .into_response(),
        ResponseFormat::VerboseJson => Json(serde_json::json!({
            "task": "transcribe",
            "language": language,
            "duration": null,
            "text": text,
            "segments": []
        }))
        .into_response(),
    }
}

fn sanitize_filename(filename: &str) -> String {
    Path::new(filename)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("audio")
        .replace('"', "")
}

/// SDKs often send `application/octet-stream` and a bare `audio` filename,
/// so the format is sniffed from the bytes when the metadata is generic.
fn normalize_upload_metadata(
    filename: &str,
    content_type: String,
    bytes: &[u8],
) -> (String, String) {
    let sniffed = sniff_audio(bytes);
    let content_type = if is_generic_content_type(&content_type) {
        sniffed
            .map(|audio| audio.content_type.to_string())
            .unwrap_or(content_type)
    } else {
        content_type
    };
    let filename = if has_recognized_audio_extension(filename) {
        filename.to_string()
    } else if let Some(audio) = sniffed.or_else(|| format_for_content_type(&content_type)) {
        let stem = Path::new(filename)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .filter(|stem| !stem.is_empty() && !stem.starts_with('.'))
            .unwrap_or("audio");
        format!("{}.{}", stem, audio.extension)
    } else {
        filename.to_string()
    };
    (filename, content_type)
}

fn sniff_audio(bytes: &[u8]) -> Option<AudioFormat> {
    let content_type = if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(&b"WAVE"[..]) {
        "audio/wav"
    } else if bytes.starts_with(b"ID3") || looks_like_mp3_frame(bytes) {
        "audio/mpeg"
    } else if bytes.starts_with(b"fLaC") {
        "audio/flac"
    } else if bytes.starts_with(b"OggS") {
        "audio/ogg"
    } else if bytes.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) {
        "audio/webm"
    } else if bytes.get(4..8) == Some(&b"ftyp"[..]) {
        "audio/mp4"
    } else if bytes.starts_with(b"#!SILK_V3") || bytes.starts_with(b"\x02#!SILK_V3") {
        "audio/silk"
    } else {
        return None;
    };
    format_for_content_type(content_type)
}

fn looks_like_mp3_frame(bytes: &[u8]) -> bool {
    matches!(bytes, [0xff, second, ..] if second & 0xe0 == 0xe0)
}

fn is_generic_content_type(content_type: &str) -> bool {
    matches!(
        content_type
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase()
            .as_str(),
        "" | "application/octet-stream" | "binary/octet-stream"
    )
}

fn has_recognized_audio_extension(filename: &str) -> bool {
    infer_content_type(filename) != "application/octet-stream" || is_silk_path(Path::new(filename))
}

fn upload_suffix(filename: &str, content_type: &str) -> String {
    if let Some(extension) = Path::new(filename)
        .extension()
        .and_then(|ext| ext.to_str())
        .filter(|ext| !ext.is_empty())
    {
        return format!(".{extension}");
    }
    let extension = format_for_content_type(content_type).map_or("bin", |format| format.extension);
    format!(".{extension}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const WAV: &[u8] = b"RIFF\x24\x00\x00\x00WAVEfmt ";

    #[test]
    fn upload_metadata_sniffs_wav_when_sdk_omits_metadata() {
        assert_eq!(
            sniff_audio(WAV).map(|audio| audio.content_type),
            Some("audio/wav")
        );
        assert_eq!(
            normalize_upload_metadata("audio", "application/octet-stream".to_string(), WAV),
            ("audio.wav".to_string(), "audio/wav".to_string())
        );
    }

    #[test]
    fn upload_metadata_keeps_explicit_audio_filename() {
        assert_eq!(
            normalize_upload_metadata("clip.wav", "application/octet-stream".to_string(), WAV),
            ("clip.wav".to_string(), "audio/wav".to_string())
        );
    }

    #[test]
    fn sniffing_covers_each_supported_container() {
        for (bytes, expected) in [
            (&b"ID3\x04"[..], "audio/mpeg"),
            (&[0xff, 0xfb, 0x90][..], "audio/mpeg"),
            (&b"fLaC"[..], "audio/flac"),
            (&b"OggS"[..], "audio/ogg"),
            (&[0x1a, 0x45, 0xdf, 0xa3][..], "audio/webm"),
            (&b"\0\0\0\x20ftypM4A "[..], "audio/mp4"),
            (&b"\x02#!SILK_V3"[..], "audio/silk"),
        ] {
            assert_eq!(sniff_audio(bytes).map(|f| f.content_type), Some(expected));
        }
        assert_eq!(sniff_audio(b"plain text"), None);
    }

    #[test]
    fn upload_suffix_prefers_the_filename_then_the_content_type() {
        assert_eq!(upload_suffix("clip.ogg", "audio/wav"), ".ogg");
        assert_eq!(upload_suffix("clip", "audio/x-silk"), ".silk");
        assert_eq!(upload_suffix("clip", "text/plain"), ".bin");
    }
}
