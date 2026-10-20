use std::env;
use std::path::PathBuf;
use std::time::Duration;

use clap::{CommandFactory, Parser, Subcommand};
use codex_voice::audio::{self, SilkConfig};
use codex_voice::{AuthSource, TranscribeOptions, TranscriptionClient};

type AnyError = Box<dyn std::error::Error + Send + Sync + 'static>;
const DEFAULT_REQUEST_TIMEOUT_SECONDS: u64 = 120;
const DEFAULT_CONNECT_TIMEOUT_SECONDS: u64 = 10;
#[cfg(feature = "server")]
const DEFAULT_MAX_UPLOAD_MIB: u64 = 10;

#[derive(Debug, Parser)]
#[command(
    version,
    about = "Transcription and realtime voice through a Codex login"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Audio file to transcribe. If no subcommand is used, this is treated as `transcribe <audio>`.
    audio: Option<PathBuf>,

    /// Bearer token. Defaults to CODEX_VOICE_BEARER or ~/.codex/auth.json.
    #[arg(long, global = true)]
    bearer: Option<String>,

    /// ChatGPT account id override. Usually decoded from the bearer token.
    #[arg(long, global = true)]
    account_id: Option<String>,

    /// Codex auth file. Defaults to $CODEX_HOME/auth.json or ~/.codex/auth.json.
    #[arg(long, global = true)]
    auth_file: Option<PathBuf>,

    /// Upstream transcribe endpoint.
    #[arg(long, global = true, default_value = audio::DEFAULT_ENDPOINT)]
    endpoint: String,

    /// HTTPS proxy URL. Defaults to CODEX_VOICE_PROXY, HTTPS_PROXY, ALL_PROXY, or macOS system proxy.
    #[arg(long, global = true)]
    proxy: Option<String>,

    /// Total upstream request timeout in seconds. Defaults to CODEX_VOICE_REQUEST_TIMEOUT_SECONDS or 120.
    #[arg(long, global = true, value_name = "SECONDS")]
    request_timeout_seconds: Option<u64>,

    /// Upstream connect timeout in seconds. Defaults to CODEX_VOICE_CONNECT_TIMEOUT_SECONDS or 10.
    #[arg(long, global = true, value_name = "SECONDS")]
    connect_timeout_seconds: Option<u64>,

    /// BCP-47-ish language hint, for example zh or en.
    #[arg(long, global = true)]
    language: Option<String>,

    /// Audio content type. Inferred from filename when omitted.
    #[arg(long, global = true)]
    content_type: Option<String>,

    /// Multipart filename override. Useful when the input path has no audio extension.
    #[arg(long, global = true)]
    filename: Option<String>,

    /// Path to an external rust-silk CLI used to decode .silk before upload.
    #[arg(long, global = true)]
    silk_decoder: Option<PathBuf>,

    /// Sample rate used when decoding .silk to temporary WAV.
    #[arg(long, global = true, default_value_t = 24_000)]
    silk_sample_rate: u32,

    /// Upload .silk as-is instead of decoding it with rust-silk.
    #[arg(long, global = true)]
    no_silk_decode: bool,

    /// Print {"text": "..."} instead of plain text.
    #[arg(long, global = true)]
    json: bool,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Transcribe an audio file.
    Transcribe {
        /// Audio file to transcribe.
        audio: PathBuf,
    },
    /// Serve the HTTP API and the demonstration web client.
    Serve(ServeArgs),
}

#[derive(Debug, clap::Args)]
struct ServeArgs {
    /// Bind host.
    #[arg(long, default_value = "127.0.0.1")]
    host: String,

    /// Bind port.
    #[arg(long, default_value_t = 2600)]
    port: u16,

    /// Local API key. Defaults to CODEX_VOICE_API_KEY.
    #[arg(long)]
    api_key: Option<String>,

    /// Allow unauthenticated requests. Only use on trusted loopback.
    #[arg(long)]
    no_api_key: bool,

    /// Maximum concurrent upstream requests.
    #[arg(long, default_value_t = 1)]
    concurrency: usize,

    /// Maximum accepted upload size in MiB. Defaults to CODEX_VOICE_MAX_UPLOAD_MB or 10.
    #[arg(long, value_name = "MIB")]
    max_upload_mb: Option<u64>,

    /// Exact Host authority (host or host:port) a private reverse proxy uses for
    /// the web client and realtime route. Repeatable. Requires a local API key.
    #[arg(long = "allowed-host", value_name = "HOST[:PORT]")]
    allowed_hosts: Vec<String>,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), AnyError> {
    let cli = Cli::parse();
    let audio = match (&cli.command, &cli.audio) {
        (Some(Command::Serve(args)), _) => return run_serve(&cli, args),
        (Some(Command::Transcribe { audio }), _) | (None, Some(audio)) => audio.clone(),
        (None, None) => {
            Cli::command().print_help()?;
            eprintln!();
            std::process::exit(2);
        }
    };
    run_transcribe(&cli, audio)
}

fn run_transcribe(cli: &Cli, audio: PathBuf) -> Result<(), AnyError> {
    let client = TranscriptionClient::builder(auth_source(cli).load()?)
        .endpoint(cli.endpoint.clone())
        .proxy(proxy(cli))
        .timeout(Some(request_timeout(cli)?))
        .connect_timeout(Some(connect_timeout(cli)?))
        .build()?;
    let decoded = audio::decode_silk(&audio, &silk_config(cli))?;
    let (path, filename, content_type) = match &decoded {
        Some(decoded) => (
            decoded.path(),
            cli.filename.clone().or(Some(decoded.filename.clone())),
            Some(audio::DecodedSilk::CONTENT_TYPE.to_string()),
        ),
        None => (
            audio.as_path(),
            cli.filename.clone(),
            cli.content_type.clone(),
        ),
    };
    let transcription = client.transcribe_file(
        path,
        TranscribeOptions {
            language: cli.language.clone(),
            content_type,
            filename,
        },
    )?;

    if cli.json {
        println!("{}", serde_json::json!({ "text": transcription.text }));
    } else {
        println!("{}", transcription.text);
    }
    Ok(())
}

fn auth_source(cli: &Cli) -> AuthSource {
    if let Some(token) = cli
        .bearer
        .clone()
        .or_else(|| env::var("CODEX_VOICE_BEARER").ok())
    {
        return AuthSource::Bearer {
            token,
            account_id: cli.account_id.clone(),
        };
    }
    AuthSource::File(
        cli.auth_file
            .clone()
            .unwrap_or_else(codex_voice::auth::default_auth_file),
    )
}

fn proxy(cli: &Cli) -> Option<String> {
    codex_voice::resolve_proxy(cli.proxy.as_deref())
}

fn silk_config(cli: &Cli) -> SilkConfig {
    SilkConfig {
        decoder: cli.silk_decoder.clone(),
        sample_rate: cli.silk_sample_rate,
        decode: !cli.no_silk_decode,
    }
}

fn request_timeout(cli: &Cli) -> Result<Duration, AnyError> {
    resolve_seconds(
        cli.request_timeout_seconds,
        "CODEX_VOICE_REQUEST_TIMEOUT_SECONDS",
        DEFAULT_REQUEST_TIMEOUT_SECONDS,
        "request timeout",
    )
}

fn connect_timeout(cli: &Cli) -> Result<Duration, AnyError> {
    resolve_seconds(
        cli.connect_timeout_seconds,
        "CODEX_VOICE_CONNECT_TIMEOUT_SECONDS",
        DEFAULT_CONNECT_TIMEOUT_SECONDS,
        "connect timeout",
    )
}

fn resolve_seconds(
    explicit: Option<u64>,
    env_name: &str,
    default: u64,
    label: &str,
) -> Result<Duration, AnyError> {
    let seconds = match explicit {
        Some(value) => value,
        None => u64_env(env_name)?.unwrap_or(default),
    };
    if seconds == 0 {
        return Err(format!("{label} must be greater than 0 seconds").into());
    }
    Ok(Duration::from_secs(seconds))
}

fn u64_env(name: &str) -> Result<Option<u64>, AnyError> {
    let Ok(value) = env::var(name) else {
        return Ok(None);
    };
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    let parsed = value
        .parse::<u64>()
        .map_err(|_| format!("{name} must be a positive integer"))?;
    Ok(Some(parsed))
}

#[cfg(feature = "server")]
fn run_serve(cli: &Cli, args: &ServeArgs) -> Result<(), AnyError> {
    use codex_voice::server::{self, ServerConfig, UpstreamConfig};

    if args.concurrency == 0 {
        return Err("--concurrency must be greater than 0".into());
    }
    let api_key = server_api_key(args)?;
    if !args.allowed_hosts.is_empty() && api_key.is_none() {
        return Err("--allowed-host requires --api-key or CODEX_VOICE_API_KEY".into());
    }
    let allowed_hosts = args
        .allowed_hosts
        .iter()
        .map(|host| parse_authority(host))
        .collect::<Result<Vec<_>, _>>()?;
    let auth = auth_source(cli);
    auth.load()?;
    let config = ServerConfig {
        api_key,
        auth,
        upstream: UpstreamConfig {
            transcribe_endpoint: cli.endpoint.clone(),
            #[cfg(feature = "realtime")]
            realtime_base: codex_voice::realtime::DEFAULT_UPSTREAM.to_string(),
            proxy: proxy(cli),
            request_timeout: request_timeout(cli)?,
            connect_timeout: connect_timeout(cli)?,
        },
        concurrency: args.concurrency,
        max_upload_bytes: max_upload_bytes(args.max_upload_mb)?,
        silk: silk_config(cli),
        default_language: cli.language.clone(),
        allowed_hosts,
    };
    let addr = format!("{}:{}", args.host, args.port);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let listener = tokio::net::TcpListener::bind(&addr)
            .await
            .map_err(|error| format!("cannot listen on {addr}: {error}"))?;
        server::serve(listener, config).await?;
        Ok(())
    })
}

#[cfg(not(feature = "server"))]
fn run_serve(_cli: &Cli, _args: &ServeArgs) -> Result<(), AnyError> {
    Err("serve is unavailable because codex-voice was built without the `server` feature".into())
}

#[cfg(feature = "server")]
fn server_api_key(args: &ServeArgs) -> Result<Option<String>, AnyError> {
    if args.no_api_key {
        return Ok(None);
    }
    args.api_key
        .clone()
        .or_else(|| env::var("CODEX_VOICE_API_KEY").ok())
        .map(|key| key.trim().to_string())
        .filter(|key| !key.is_empty())
        .map(Some)
        .ok_or_else(|| "serve requires --api-key, CODEX_VOICE_API_KEY, or --no-api-key".into())
}

/// Accepts only a bare `host` or `host:port`, as browsers send in `Host`.
#[cfg(feature = "server")]
fn parse_authority(value: &str) -> Result<String, AnyError> {
    let invalid = || format!("--allowed-host must be host or host:port, got {value:?}");
    if !value
        .bytes()
        .all(|c| c.is_ascii_alphanumeric() || b".-:[]".contains(&c))
    {
        return Err(invalid().into());
    }
    let url = reqwest::Url::parse(&format!("http://{value}")).map_err(|_| invalid())?;
    let authority = url.authority().to_ascii_lowercase();
    if url.host_str().is_none()
        || url.path() != "/"
        || !url.username().is_empty()
        || authority != value.to_ascii_lowercase()
    {
        return Err(invalid().into());
    }
    Ok(authority)
}

#[cfg(feature = "server")]
fn max_upload_bytes(explicit_mib: Option<u64>) -> Result<usize, AnyError> {
    let mib = match explicit_mib {
        Some(value) => value,
        None => u64_env("CODEX_VOICE_MAX_UPLOAD_MB")?.unwrap_or(DEFAULT_MAX_UPLOAD_MIB),
    };
    if mib == 0 {
        return Err("max upload size must be greater than 0 MiB".into());
    }
    mib.checked_mul(1024 * 1024)
        .and_then(|bytes| usize::try_from(bytes).ok())
        .ok_or_else(|| "max upload size is too large".into())
}

#[cfg(all(test, feature = "server"))]
mod tests {
    use super::parse_authority;

    #[test]
    fn allowed_hosts_must_be_exact_authorities() {
        assert_eq!(parse_authority("192.168.1.100").unwrap(), "192.168.1.100");
        assert_eq!(parse_authority("Voice.LAN:8443").unwrap(), "voice.lan:8443");
        assert_eq!(parse_authority("[::1]:2600").unwrap(), "[::1]:2600");
        for bad in [
            "*",
            "*.lan",
            "https://voice.lan",
            "voice.lan/path",
            "user@voice.lan",
            "",
            "voice.lan:x",
        ] {
            assert!(parse_authority(bad).is_err(), "{bad}");
        }
    }
}
