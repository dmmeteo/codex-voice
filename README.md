# Codex Voice

Codex Voice is a Rust library, CLI, and HTTP server for file transcription and
WebRTC voice signaling through a server-side Codex ChatGPT login. The optional
browser demo is a small static JavaScript client of the same HTTP API.

This project uses private ChatGPT services. It does not implement OpenAI's public
Realtime API or GPT-Live, and it does not establish a working Discord voice
integration. Private-service access and behavior can change independently of
this code. Upstream codex-asr releases and images are not Codex Voice releases.

## Run from source

The repository pins Rust 1.98.0. Default features are `server` and `realtime`.
The native realtime transport requires CMake, Clang/libclang, and pkg-config.
These are source-build instructions, not a promise of published binaries or images.

With `CODEX_VOICE_API_KEY` supplied through your external server environment:

```sh
cargo run --locked -- serve
```

The server defaults to `127.0.0.1:2600`, concurrency 1, a 10 MiB multipart request
limit, and upstream request/connect timeouts of 120/10 seconds. Bind address and
port remain configurable with `--host` and `--port`. Open
`http://127.0.0.1:2600/` and enter the local server key to use the demo.

`serve` requires `--api-key` or `CODEX_VOICE_API_KEY`. To disable local key checks
explicitly on trusted loopback:

```sh
cargo run --locked -- serve --host 127.0.0.1 --port 2600 --no-api-key
```

For a file transcription without the HTTP server:

```sh
cargo run --locked -- transcribe recording.wav --language en --json
```

Without `--json`, the CLI prints plain text. `--content-type` and `--filename`
override upload metadata. SILK files are decoded through a separate `rust-silk`
executable, configured by `--silk-decoder` or `CODEX_VOICE_SILK_DECODER`.

## Authentication and configuration

Upstream Codex authentication and the local server key are separate.
The server reads its existing Codex ChatGPT login. `--auth-file` overrides the
default login location selected by `CODEX_HOME`. `--bearer` or
`CODEX_VOICE_BEARER` takes precedence over file auth. File auth is reloaded for
each transcription or negotiation. The server does not refresh the login.
Keep credentials and their files outside the checkout and browser.

HTTP clients send the local key as `Authorization: Bearer <local-key>` when
configured. This is not the upstream Codex token or an OpenAI project key.
The demo keeps the local key in its input field and does not receive upstream
tokens or private status tickets.

| Setting | CLI flag | Environment variable | Default |
| --- | --- | --- | --- |
| Local server key | `--api-key` | `CODEX_VOICE_API_KEY` | Required unless `--no-api-key` |
| Upstream concurrency | `--concurrency` | None | `1` |
| Multipart body limit, MiB | `--max-upload-mb` | `CODEX_VOICE_MAX_UPLOAD_MB` | `10` |
| Upstream request timeout, seconds | `--request-timeout-seconds` | `CODEX_VOICE_REQUEST_TIMEOUT_SECONDS` | `120` |
| Upstream connect timeout, seconds | `--connect-timeout-seconds` | `CODEX_VOICE_CONNECT_TIMEOUT_SECONDS` | `10` |
| Upstream proxy | `--proxy` | `CODEX_VOICE_PROXY`, then standard proxy variables | macOS system HTTPS proxy if available |

Explicit flags override the corresponding environment values. Use the timeout
and proxy flags before `serve`. `--language` sets a transcription default, not
the realtime voice language.

### Use a keyed private reverse proxy

Repeat `--allowed-host HOST[:PORT]` for exact browser-facing authorities in
addition to the bind address. For example, with the local key already supplied
by the external environment:

```sh
cargo run --locked -- serve --host 127.0.0.1 --port 2600 \
	--allowed-host voice.example --allowed-host voice.example:8443
```

`voice.example` is a placeholder, not a deployed address. Entries contain no
scheme, path, user info, or wildcard. This option requires a local key and cannot
widen keyless access. The proxy must preserve the allowlisted Host authority and
the browser's Origin. The Origin authority must match Host. The server does not
trust `Forwarded` or `X-Forwarded-Host` to bypass those checks.

Browsers need HTTPS or localhost for microphone access. Plain HTTP on a remote
LAN address is insufficient. Serve the demo and API through the same trusted
HTTPS origin, and enter the local key in the demo. Allowlisting an authority
does not configure TLS, proxy access control, or network exposure.

## HTTP endpoints

`server` enables the following routes. Session signaling additionally requires
`realtime`. The demo and static assets are compiled into the server binary.

| Method and path | Request | Response and access |
| --- | --- | --- |
| `GET /healthz` | No body | `{"status":"ok"}`, no key check. Does not probe upstream readiness. |
| `POST /v1/audio/transcriptions` | Multipart audio upload | Transcript, local key when configured. Cross-site browser requests are rejected. |
| `POST /v1/realtime/sessions` | JSON audio SDP offer and options | JSON answer and correlation ID, local key when configured, Host/Origin checks. |
| `GET /` | No body | Static HTML demo, Host/Origin checks, no key check. |
| `GET /index.js`, `GET /transcriptions.js`, `GET /realtime.js`, `GET /styles.css` | No body | Static assets with the same checks as the HTML. |

The historical `/audio/transcriptions` alias, `/v1/voice/sessions`,
`/v1/voice/config`, and `/voice` page are removed. There are no demo configuration,
event polling, documentation, or REST pause/stop endpoints. An endpoint list
still describes HTTP signaling even though WebRTC carries the ongoing media.
A static OpenAPI file is a future option, not an implemented endpoint.

### Transcribe an audio file

`file` is required and nonempty. `language` is a hint forwarded upstream and
falls back to the server's `--language` value when omitted. `response_format`
defaults to `json`, returning `{"text":"..."}`. `text` returns plain UTF-8 text.
`verbose_json` returns `task`, the requested `language`, `duration: null`, `text`,
and `segments: []`. Timestamps, `srt`, and `vtt` are unsupported.

`model`, `prompt`, `temperature`, timestamp granularity, and other unrecognized
multipart fields are ignored. A `model` value such as the examples' `whisper-1`
is for client request shape only. It does not select the actual backend.
The 10 MiB default covers the whole multipart body, including framing and fields.
It does not guarantee room for a full 10 MiB audio file plus overhead.

Given a local `recording.wav` and a local key supplied in the client environment:

```sh
curl --fail-with-body http://127.0.0.1:2600/v1/audio/transcriptions \
	-H "Authorization: Bearer $CODEX_VOICE_API_KEY" \
	-F 'file=@recording.wav;type=audio/wav' \
	-F language=en -F response_format=json
```

Omit Authorization only for an explicitly keyless server. A successful response
has this shape:

```json
{"text":"Recognized speech."}
```

The [curl](examples/curl.sh), [Python](examples/python_openai_sdk.py), and
[Node](examples/node_openai_sdk.mjs) examples target this file-upload contract.
They use `CODEX_VOICE_BASE_URL` with `http://127.0.0.1:2600/v1` as the default,
and `CODEX_VOICE_API_KEY`. Install their client dependencies separately.

### Negotiate private realtime voice

`POST /v1/realtime/sessions` accepts `application/json` with these fields:

| JSON field | Type and default |
| --- | --- |
| `sdp` | Required complete WebRTC audio offer, at most 128 KiB. |
| `mode` | `standard`, `advanced`, or `wingman`. API default `advanced`. |
| `voice` | Identifier of 1 to 64 ASCII letters, digits, underscores, or hyphens. Default `cove`. |
| `language` | Optional language tag such as `en-US`, at most 35 characters. Omitted or `null` means automatic detection. |
| `enable_message_streaming` | Boolean, default `true`. Requests user and assistant text events. |
| `history_and_training_disabled` | Boolean, default `true`. `false` permits saving to ChatGPT history. |
| `timezone` | Timezone name, default `UTC`. |
| `timezone_offset_minutes` | Integer minutes west of UTC, from -840 to 840. Default `0`, matching JavaScript `getTimezoneOffset()`. |

Unknown fields are rejected. Create `offer.json` with your peer's complete SDP
in place of the placeholder:

```json
{"sdp":"<your peer's complete SDP>","mode":"standard","history_and_training_disabled":true}
```

```sh
curl --fail-with-body http://127.0.0.1:2600/v1/realtime/sessions \
	-H "Authorization: Bearer $CODEX_VOICE_API_KEY" \
	-H 'Content-Type: application/json' --data-binary @offer.json
```

The response is `{"sdp":"<answer SDP>","session_id":"<correlation ID>"}`.
The placeholders are not usable SDP. Apply the answer to the originating peer.
Audio and events then flow over WebRTC and its data channel. `session_id` is
correlation, not a server-owned media resource. The client owns microphone,
playback, Pause/Resume, and Stop cleanup. HTTP success alone does not prove a
working conversation. See the [voice guide](docs/voice.md) for lifecycle details.

Application errors use `{"error":{"message":"...","type":"...","param":null,"code":null}}`.
Typical statuses are 400 for malformed options, 401 for a missing/invalid local
key, 403 for browser-origin checks, 413 for body/SDP limits, 429 for busy signaling
or upstream limits, 502 for upstream failure, and 504 for realtime upstream
timeout. Framework rejections may have different bodies.

## Rust library and integration limits

The crate is `codex_voice`. `audio::TranscriptionClient` handles file uploads.
`realtime::RealtimeClient` negotiates SDP asynchronously. Both use shared
`CodexAuth`; the realtime caller supplies a runtime and WebRTC media handling.
Disable default features for an audio-only library. Enable only `server` for an
ASR-only server, or only `realtime` for signaling without Axum.

The demo has Transcriptions and Realtime views. File upload and microphone
recording use the transcription API. Realtime has Transcript/Events output,
explicit history saving, and local Pause/Resume/Stop controls. It defaults to
Standard with saving off. Advanced and Wingman require explicit saving in the
UI. The JSON handler forwards either history setting without that UI restriction.

Modes, requested model fields, and a reported model slug do not prove the actual
served voice model. There is no verified `gptlive-1.1` selector or voice pricing
claim. Codex Voice does not provide native Hermes GPT-Live delegation events.
Hermes file STT can use the multipart compatibility seam. Native GPT-Live and
Discord media integration need separate transport work and live verification.
The [voice guide](docs/voice.md) describes those boundaries and the STT comparison
procedure. Mock tests do not establish real microphone permission or live audio.

## License

[MIT](LICENSE-MIT). Codex Voice derives from codex-asr. The inherited copyright
and full permission notice remain unchanged in the license.
