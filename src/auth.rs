//! Codex credentials for upstream requests and the local API key for this server.
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use base64::prelude::*;
use serde::Deserialize;
use serde_json::Value;
use thiserror::Error;

/// Upstream services expect requests that identify as Codex Desktop.
pub const ORIGINATOR: &str = "Codex Desktop";
const DEFAULT_DESKTOP_VERSION: &str = "26.429.30905";

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("failed to read {path}: {source}")]
    ReadFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("Codex auth at {path} does not contain a ChatGPT access token")]
    MissingAccessToken { path: PathBuf },
    #[error("Codex auth mode is {mode}, not ChatGPT token auth")]
    UnsupportedAuthMode { mode: String },
    #[error("invalid bearer token")]
    InvalidBearer,
}

#[derive(Clone)]
pub struct CodexAuth {
    pub access_token: String,
    pub account_id: Option<String>,
    pub path: Option<PathBuf>,
}

impl std::fmt::Debug for CodexAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CodexAuth")
            .field("access_token", &"[redacted]")
            .field(
                "account_id",
                &self.account_id.as_ref().map(|_| "[redacted]"),
            )
            .field("path", &self.path)
            .finish()
    }
}

impl CodexAuth {
    pub fn from_bearer(
        token: impl AsRef<str>,
        account_id: Option<String>,
    ) -> Result<Self, AuthError> {
        let access_token = strip_bearer_prefix(token.as_ref()).ok_or(AuthError::InvalidBearer)?;
        let account_id = account_id.or_else(|| account_id_from_access_token(&access_token));
        Ok(Self {
            access_token,
            account_id,
            path: None,
        })
    }

    pub fn from_codex_home() -> Result<Self, AuthError> {
        Self::from_auth_file(default_auth_file())
    }

    pub fn from_auth_file(path: impl AsRef<Path>) -> Result<Self, AuthError> {
        let path = path.as_ref().to_path_buf();
        let raw = fs::read_to_string(&path).map_err(|source| AuthError::ReadFile {
            path: path.clone(),
            source,
        })?;
        let parsed: AuthFile = serde_json::from_str(&raw).map_err(|source| AuthError::Parse {
            path: path.clone(),
            source,
        })?;
        if let Some(mode) = parsed.auth_mode.or(parsed.auth_mode_camel) {
            if mode != "chatgpt" && mode != "chatgpt_auth_tokens" {
                return Err(AuthError::UnsupportedAuthMode { mode });
            }
        }
        let tokens = parsed.tokens;
        let token = tokens
            .as_ref()
            .and_then(|tokens| tokens.access_token.clone())
            .filter(|token| !token.trim().is_empty())
            .ok_or_else(|| AuthError::MissingAccessToken { path: path.clone() })?;
        let account_id = tokens
            .and_then(|tokens| tokens.account_id)
            .or_else(|| account_id_from_access_token(&token));
        Ok(Self {
            access_token: token,
            account_id,
            path: Some(path),
        })
    }
}

/// Where the server reads Codex credentials. A file is re-read for every
/// upstream request so a refreshed Codex login applies without a restart.
#[derive(Debug, Clone)]
pub enum AuthSource {
    Bearer {
        token: String,
        account_id: Option<String>,
    },
    File(PathBuf),
}

impl AuthSource {
    pub fn load(&self) -> Result<CodexAuth, AuthError> {
        match self {
            Self::Bearer { token, account_id } => CodexAuth::from_bearer(token, account_id.clone()),
            Self::File(path) => CodexAuth::from_auth_file(path),
        }
    }
}

#[derive(Debug, Deserialize)]
struct AuthFile {
    auth_mode: Option<String>,
    #[serde(rename = "authMode")]
    auth_mode_camel: Option<String>,
    tokens: Option<AuthTokens>,
}

#[derive(Debug, Deserialize)]
struct AuthTokens {
    access_token: Option<String>,
    account_id: Option<String>,
}

pub fn default_auth_file() -> PathBuf {
    let codex_home = env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home_dir().join(".codex"));
    codex_home.join("auth.json")
}

/// The User-Agent sent upstream alongside the `originator` header.
pub fn desktop_user_agent() -> String {
    let version =
        detect_codex_desktop_version().unwrap_or_else(|| DEFAULT_DESKTOP_VERSION.to_string());
    format!(
        "{ORIGINATOR}/{version} ({}; {})",
        env::consts::OS,
        env::consts::ARCH
    )
}

/// Accepts `Authorization: Bearer <key>` when the server has a local API key.
pub fn is_authorized(authorization: Option<&str>, expected: Option<&str>) -> bool {
    let Some(expected) = expected else {
        return true;
    };
    authorization
        .and_then(|value| {
            value
                .strip_prefix("Bearer ")
                .or_else(|| value.strip_prefix("bearer "))
        })
        .is_some_and(|provided| provided == expected)
}

fn strip_bearer_prefix(token: &str) -> Option<String> {
    let trimmed = token.trim();
    let token = trimmed
        .strip_prefix("Bearer ")
        .or_else(|| trimmed.strip_prefix("bearer "))
        .unwrap_or(trimmed)
        .trim();
    (!token.is_empty()).then(|| token.to_string())
}

fn account_id_from_access_token(access_token: &str) -> Option<String> {
    let payload = access_token.split('.').nth(1)?;
    let decoded = BASE64_URL_SAFE_NO_PAD.decode(payload).ok()?;
    let value: Value = serde_json::from_slice(&decoded).ok()?;
    value
        .get("https://api.openai.com/auth")?
        .get("chatgpt_account_id")?
        .as_str()
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn detect_codex_desktop_version() -> Option<String> {
    let plist = fs::read_to_string("/Applications/Codex.app/Contents/Info.plist").ok()?;
    let marker = "<key>CFBundleShortVersionString</key>";
    let rest = plist.split_once(marker)?.1;
    let start = rest.find("<string>")? + "<string>".len();
    let end = rest[start..].find("</string>")?;
    Some(rest[start..start + end].to_string())
}

fn home_dir() -> PathBuf {
    env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bearer_prefix_is_optional() {
        assert_eq!(
            strip_bearer_prefix("Bearer abc.def").as_deref(),
            Some("abc.def")
        );
        assert_eq!(strip_bearer_prefix("abc.def").as_deref(), Some("abc.def"));
        assert_eq!(strip_bearer_prefix("  ").as_deref(), None);
    }

    #[test]
    fn account_id_can_be_read_from_chatgpt_jwt_payload() {
        let payload = BASE64_URL_SAFE_NO_PAD
            .encode(r#"{"https://api.openai.com/auth":{"chatgpt_account_id":"acct_123"}}"#);
        let token = format!("header.{payload}.sig");
        assert_eq!(
            account_id_from_access_token(&token).as_deref(),
            Some("acct_123")
        );
    }

    #[test]
    fn local_api_key_requires_exact_bearer_match() {
        assert!(is_authorized(None, None));
        assert!(is_authorized(Some("Bearer key"), Some("key")));
        assert!(is_authorized(Some("bearer key"), Some("key")));
        assert!(!is_authorized(Some("Bearer other"), Some("key")));
        assert!(!is_authorized(Some("key"), Some("key")));
        assert!(!is_authorized(None, Some("key")));
    }
}
