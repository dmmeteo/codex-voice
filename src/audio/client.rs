//! Upstream transcription through the ChatGPT transcribe endpoint.
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use reqwest::blocking::{multipart, Client};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, USER_AGENT};
use serde::Deserialize;
use thiserror::Error;

use crate::auth::{desktop_user_agent, CodexAuth, ORIGINATOR};

pub const DEFAULT_ENDPOINT: &str = "https://chatgpt.com/backend-api/transcribe";

#[derive(Debug, Error)]
pub enum TranscriptionError {
    #[error("failed to read {path}: {source}")]
    ReadFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid header value: {0}")]
    InvalidHeader(#[from] reqwest::header::InvalidHeaderValue),
    #[error("failed to build HTTP client: {0}")]
    BuildClient(#[source] reqwest::Error),
    #[error("failed to build multipart request: {0}")]
    BuildMultipart(#[source] reqwest::Error),
    #[error("transcribe request failed: {0}")]
    Request(#[source] reqwest::Error),
    #[error("transcribe request failed with HTTP {status}: {body}")]
    Http { status: u16, body: String },
    #[error("failed to parse transcribe response: {0}")]
    InvalidResponse(#[source] serde_json::Error),
    #[error("transcribe response did not contain text")]
    MissingText,
}

#[derive(Debug, Clone)]
pub struct TranscriptionClient {
    endpoint: String,
    auth: CodexAuth,
    http: Client,
    user_agent: String,
}

impl TranscriptionClient {
    pub fn builder(auth: CodexAuth) -> TranscriptionClientBuilder {
        TranscriptionClientBuilder::new(auth)
    }

    pub fn transcribe_file(
        &self,
        path: impl AsRef<Path>,
        options: TranscribeOptions,
    ) -> Result<Transcription, TranscriptionError> {
        let path = path.as_ref();
        let audio = fs::read(path).map_err(|source| TranscriptionError::ReadFile {
            path: path.to_path_buf(),
            source,
        })?;
        let content_type = options
            .content_type
            .unwrap_or_else(|| infer_content_type(path).to_string());
        let filename = options
            .filename
            .unwrap_or_else(|| upload_filename(path, &content_type));
        self.transcribe_bytes(audio, &filename, &content_type, options.language)
    }

    pub fn transcribe_bytes(
        &self,
        audio: Vec<u8>,
        filename: &str,
        content_type: &str,
        language: Option<String>,
    ) -> Result<Transcription, TranscriptionError> {
        let part = multipart::Part::bytes(audio)
            .file_name(filename.replace('"', ""))
            .mime_str(content_type)
            .map_err(TranscriptionError::BuildMultipart)?;
        let mut form = multipart::Form::new().part("file", part);
        if let Some(language) = language {
            form = form.text("language", language);
        }

        let response = self
            .http
            .post(&self.endpoint)
            .headers(self.auth_headers()?)
            .multipart(form)
            .send()
            .map_err(TranscriptionError::Request)?;
        let status = response.status();
        let body = response.text().map_err(TranscriptionError::Request)?;
        if !status.is_success() {
            return Err(TranscriptionError::Http {
                status: status.as_u16(),
                body: clip_response_body(&body),
            });
        }
        let parsed: TranscribeResponse =
            serde_json::from_str(&body).map_err(TranscriptionError::InvalidResponse)?;
        let text = parsed.text.ok_or(TranscriptionError::MissingText)?;
        Ok(Transcription { text })
    }

    fn auth_headers(&self) -> Result<HeaderMap, TranscriptionError> {
        let mut headers = HeaderMap::new();
        let mut bearer = HeaderValue::from_str(&format!("Bearer {}", self.auth.access_token))?;
        bearer.set_sensitive(true);
        headers.insert(AUTHORIZATION, bearer);
        headers.insert("originator", HeaderValue::from_static(ORIGINATOR));
        headers.insert(USER_AGENT, HeaderValue::from_str(&self.user_agent)?);
        if let Some(account_id) = &self.auth.account_id {
            headers.insert("ChatGPT-Account-Id", HeaderValue::from_str(account_id)?);
        }
        Ok(headers)
    }
}

#[derive(Debug, Clone)]
pub struct TranscriptionClientBuilder {
    auth: CodexAuth,
    endpoint: String,
    proxy: Option<String>,
    timeout: Option<Duration>,
    connect_timeout: Option<Duration>,
    user_agent: String,
}

impl TranscriptionClientBuilder {
    pub fn new(auth: CodexAuth) -> Self {
        Self {
            auth,
            endpoint: DEFAULT_ENDPOINT.to_string(),
            proxy: crate::resolve_proxy(None),
            timeout: None,
            connect_timeout: None,
            user_agent: desktop_user_agent(),
        }
    }

    pub fn endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = endpoint.into();
        self
    }

    pub fn proxy(mut self, proxy: Option<String>) -> Self {
        self.proxy = proxy;
        self
    }

    pub fn timeout(mut self, timeout: Option<Duration>) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn connect_timeout(mut self, timeout: Option<Duration>) -> Self {
        self.connect_timeout = timeout;
        self
    }

    pub fn user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.user_agent = user_agent.into();
        self
    }

    pub fn build(self) -> Result<TranscriptionClient, TranscriptionError> {
        let mut builder = Client::builder();
        if let Some(proxy) = self.proxy {
            builder = builder
                .proxy(reqwest::Proxy::https(&proxy).map_err(TranscriptionError::BuildClient)?);
        }
        if let Some(timeout) = self.timeout {
            builder = builder.timeout(timeout);
        }
        if let Some(timeout) = self.connect_timeout {
            builder = builder.connect_timeout(timeout);
        }
        let http = builder.build().map_err(TranscriptionError::BuildClient)?;
        Ok(TranscriptionClient {
            endpoint: self.endpoint,
            auth: self.auth,
            http,
            user_agent: self.user_agent,
        })
    }
}

#[derive(Debug, Clone, Default)]
pub struct TranscribeOptions {
    pub language: Option<String>,
    pub content_type: Option<String>,
    pub filename: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transcription {
    pub text: String,
}

#[derive(Debug, Deserialize)]
struct TranscribeResponse {
    text: Option<String>,
}

pub fn infer_content_type(path: impl AsRef<Path>) -> &'static str {
    match path
        .as_ref()
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "wav" | "wave" => "audio/wav",
        "webm" => "audio/webm",
        "mp3" => "audio/mpeg",
        "m4a" | "mp4" => "audio/mp4",
        "ogg" | "oga" => "audio/ogg",
        "flac" => "audio/flac",
        _ => "application/octet-stream",
    }
}

fn upload_filename(path: &Path, content_type: &str) -> String {
    let original = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("codex")
        .to_string();
    if infer_content_type(path) != "application/octet-stream" {
        return original;
    }
    let Some(format) =
        super::format_for_content_type(content_type).filter(|format| format.extension != "silk")
    else {
        return original;
    };
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty() && !stem.starts_with('.'))
        .unwrap_or("codex");
    format!("{stem}.{}", format.extension)
}

fn clip_response_body(body: &str) -> String {
    let mut clipped = body.split_whitespace().collect::<Vec<_>>().join(" ");
    clipped.truncate(300);
    clipped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_type_is_inferred_from_extension() {
        assert_eq!(infer_content_type("voice.wav"), "audio/wav");
        assert_eq!(infer_content_type("voice.webm"), "audio/webm");
        assert_eq!(
            infer_content_type("voice.unknown"),
            "application/octet-stream"
        );
    }

    #[test]
    fn upload_filename_uses_content_type_when_extension_is_unknown() {
        assert_eq!(
            upload_filename(Path::new("voice"), "audio/wav"),
            "voice.wav"
        );
        assert_eq!(
            upload_filename(Path::new("voice.bin"), "audio/webm"),
            "voice.webm"
        );
        assert_eq!(
            upload_filename(Path::new("voice.wav"), "audio/webm"),
            "voice.wav"
        );
        assert_eq!(
            upload_filename(Path::new("voice.bin"), "application/octet-stream"),
            "voice.bin"
        );
        assert_eq!(
            upload_filename(Path::new("voice.bin"), "audio/silk"),
            "voice.bin"
        );
    }
}
