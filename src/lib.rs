//! Codex Voice: transcription through a Codex login.
//!
//! [`audio`] uploads recorded audio for transcription. [`server`] exposes it
//! over HTTP.
use std::env;
use std::process::Command;

pub mod audio;
pub mod auth;
#[cfg(feature = "server")]
pub mod server;

pub use audio::{
    TranscribeOptions, Transcription, TranscriptionClient, TranscriptionClientBuilder,
    TranscriptionError,
};
pub use auth::{AuthError, AuthSource, CodexAuth};

/// Resolves the HTTPS proxy: the explicit value, then `CODEX_VOICE_PROXY`,
/// the standard proxy variables, then the macOS system proxy.
pub fn resolve_proxy(explicit_proxy: Option<&str>) -> Option<String> {
    first_non_empty([
        explicit_proxy.map(str::to_string),
        env::var("CODEX_VOICE_PROXY").ok(),
        env::var("HTTPS_PROXY").ok(),
        env::var("https_proxy").ok(),
        env::var("ALL_PROXY").ok(),
        env::var("all_proxy").ok(),
        system_https_proxy(),
    ])
}

fn first_non_empty(values: impl IntoIterator<Item = Option<String>>) -> Option<String> {
    values
        .into_iter()
        .flatten()
        .map(|value| value.trim().to_string())
        .find(|value| !value.is_empty())
}

fn system_https_proxy() -> Option<String> {
    if cfg!(target_os = "macos") {
        let output = Command::new("scutil").arg("--proxy").output().ok()?;
        if !output.status.success() {
            return None;
        }
        return parse_scutil_https_proxy(&String::from_utf8_lossy(&output.stdout));
    }
    None
}

fn parse_scutil_https_proxy(output: &str) -> Option<String> {
    let mut enabled = false;
    let mut host = None;
    let mut port = None;
    for line in output.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        match key.trim() {
            "HTTPSEnable" => enabled = value.trim() == "1",
            "HTTPSProxy" => host = Some(value.trim().to_string()),
            "HTTPSPort" => port = Some(value.trim().to_string()),
            _ => {}
        }
    }
    if enabled {
        Some(format!("http://{}:{}", host?, port?))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macos_https_proxy_is_parsed() {
        let output = r#"
<dictionary> {
  HTTPSEnable : 1
  HTTPSProxy : 127.0.0.1
  HTTPSPort : 7892
}
"#;
        assert_eq!(
            parse_scutil_https_proxy(output).as_deref(),
            Some("http://127.0.0.1:7892")
        );
    }
}
