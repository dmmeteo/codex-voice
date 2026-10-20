//! Audio transcription: the upstream client, `.silk` decoding before upload,
//! and the `POST /v1/audio/transcriptions` route.
use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::NamedTempFile;
use thiserror::Error;

mod client;
#[cfg(feature = "server")]
pub(crate) mod routes;

pub use client::{
    infer_content_type, TranscribeOptions, Transcription, TranscriptionClient,
    TranscriptionClientBuilder, TranscriptionError, DEFAULT_ENDPOINT,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AudioFormat {
    pub content_type: &'static str,
    pub extension: &'static str,
}

pub(crate) fn format_for_content_type(content_type: &str) -> Option<AudioFormat> {
    let (content_type, extension) = match content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "audio/wav" | "audio/x-wav" | "audio/wave" => ("audio/wav", "wav"),
        "audio/mpeg" | "audio/mp3" => ("audio/mpeg", "mp3"),
        "audio/mp4" | "audio/m4a" | "audio/x-m4a" => ("audio/mp4", "m4a"),
        "audio/flac" | "audio/x-flac" => ("audio/flac", "flac"),
        "audio/ogg" => ("audio/ogg", "ogg"),
        "audio/webm" => ("audio/webm", "webm"),
        "audio/silk" | "audio/x-silk" => ("audio/silk", "silk"),
        _ => return None,
    };
    Some(AudioFormat {
        content_type,
        extension,
    })
}

#[derive(Debug, Error)]
pub enum SilkError {
    #[error("{0}")]
    Decoder(String),
    #[error("rust-silk decode failed: {0}")]
    Io(#[from] std::io::Error),
}

/// How `.silk` input is handled before upload. Upstream does not accept SILK,
/// so it is decoded to WAV with an external `rust-silk` CLI unless disabled.
#[derive(Debug, Clone)]
pub struct SilkConfig {
    pub decoder: Option<PathBuf>,
    pub sample_rate: u32,
    pub decode: bool,
}

impl Default for SilkConfig {
    fn default() -> Self {
        Self {
            decoder: None,
            sample_rate: 24_000,
            decode: true,
        }
    }
}

/// A decoded WAV that lives as long as this value.
pub struct DecodedSilk {
    file: NamedTempFile,
    pub filename: String,
}

impl DecodedSilk {
    pub const CONTENT_TYPE: &'static str = "audio/wav";

    pub fn path(&self) -> &Path {
        self.file.path()
    }
}

/// Decodes `audio` when it is a `.silk` file and decoding is enabled.
pub fn decode_silk(audio: &Path, config: &SilkConfig) -> Result<Option<DecodedSilk>, SilkError> {
    if !config.decode || !is_silk_path(audio) {
        return Ok(None);
    }
    let decoder = resolve_silk_decoder(config.decoder.as_deref())?;
    let output = tempfile::Builder::new()
        .prefix("codex-voice-silk-")
        .suffix(".wav")
        .tempfile()?;
    let status = Command::new(&decoder)
        .arg("decode")
        .arg("-i")
        .arg(audio)
        .arg("-o")
        .arg(output.path())
        .arg("--sample-rate")
        .arg(config.sample_rate.to_string())
        .arg("--quiet")
        .output()?;
    if !status.status.success() {
        let stderr = String::from_utf8_lossy(&status.stderr);
        let stdout = String::from_utf8_lossy(&status.stdout);
        let detail = [stderr.trim(), stdout.trim()]
            .into_iter()
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        return Err(SilkError::Decoder(format!(
            "rust-silk decode failed with status {}{}",
            status.status,
            if detail.is_empty() {
                String::new()
            } else {
                format!(": {detail}")
            }
        )));
    }
    if output.as_file().metadata()?.len() <= 44 {
        return Err(SilkError::Decoder(
            "rust-silk decode produced an empty WAV".into(),
        ));
    }
    Ok(Some(DecodedSilk {
        file: output,
        filename: decoded_wav_filename(audio),
    }))
}

pub(crate) fn is_silk_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| matches!(ext.to_ascii_lowercase().as_str(), "silk" | "slk"))
}

fn decoded_wav_filename(path: &Path) -> String {
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty() && !stem.starts_with('.'))
        .unwrap_or("voice");
    format!("{stem}.wav")
}

fn resolve_silk_decoder(explicit: Option<&Path>) -> Result<PathBuf, SilkError> {
    if let Some(path) = explicit {
        if path.exists() {
            return Ok(path.to_path_buf());
        }
        return Err(SilkError::Decoder(format!(
            "rust-silk decoder not found at {}",
            path.display()
        )));
    }
    if let Ok(path) = env::var("CODEX_VOICE_SILK_DECODER") {
        let path = PathBuf::from(path);
        if path.exists() {
            return Ok(path);
        }
        return Err(SilkError::Decoder(format!(
            "CODEX_VOICE_SILK_DECODER points to missing file {}",
            path.display()
        )));
    }
    if let Some(path) = find_on_path("rust-silk") {
        return Ok(path);
    }
    Err(SilkError::Decoder(
        "input is .silk, but rust-silk was not found; pass --silk-decoder or set CODEX_VOICE_SILK_DECODER".into(),
    ))
}

fn find_on_path(binary: &str) -> Option<PathBuf> {
    let paths = env::var_os("PATH")?;
    env::split_paths(&paths)
        .map(|dir| dir.join(binary))
        .find(|path| path.exists())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silk_paths_are_detected_case_insensitively() {
        assert!(is_silk_path(Path::new("voice.silk")));
        assert!(is_silk_path(Path::new("voice.SLK")));
        assert!(!is_silk_path(Path::new("voice.wav")));
    }

    #[test]
    fn decoded_wav_filename_uses_original_stem() {
        assert_eq!(
            decoded_wav_filename(Path::new("/tmp/example.silk")),
            "example.wav"
        );
        assert_eq!(decoded_wav_filename(Path::new(".silk")), "voice.wav");
    }

    #[test]
    fn non_silk_input_and_disabled_decoding_pass_through() {
        let config = SilkConfig {
            decoder: Some("/missing/rust-silk".into()),
            ..SilkConfig::default()
        };
        assert!(decode_silk(Path::new("clip.wav"), &config)
            .unwrap()
            .is_none());
        let disabled = SilkConfig {
            decode: false,
            ..config.clone()
        };
        assert!(decode_silk(Path::new("clip.silk"), &disabled)
            .unwrap()
            .is_none());
        assert_eq!(
            decode_silk(Path::new("clip.silk"), &config)
                .err()
                .map(|error| error.to_string()),
            Some("rust-silk decoder not found at /missing/rust-silk".into())
        );
    }
}
