# Codex Voice clients and integration boundaries

Codex Voice negotiates private ChatGPT voice sessions and uploads files for
transcription. The Rust server does not relay ongoing WebRTC audio or implement
native Hermes GPT-Live delegation. The repository README contains the supported
HTTP routes, auth, configuration, and request schemas. This guide may also be
copied into a local package as VOICE.md; an adjacent package README is its launcher
quickstart, not the repository's full API reference.

## Use the static demo

1. Start the server using the repository README or an existing local package's
   launcher instructions. Source defaults are loopback port 2600.
2. Open `http://127.0.0.1:2600/`. Enter the local server key when configured.
   Remote browser microphone use needs trusted HTTPS and an exact keyed
   `--allowed-host` authority, as described in the repository README.
3. In **Transcriptions**, choose a file and click **Transcribe file**, or click
   **Record** and allow microphone access. **Stop and transcribe** ends recording
   and uploads the file. During a pending request, **Stop** cancels it. The result
   appears on the right.
4. In **Realtime**, choose a mode and voice. Standard and `cove` are defaults.
   Leave **Save conversation** off for temporary history. The current UI requires
   that explicit checkbox for Advanced and Wingman. **Transcripts** requests text
   independently of saving.
5. Click **Start**, allow microphone access, and wait for **Active**. View the
   **Transcript** or **Events** output on the right. HTTP success alone does not
   establish readiness.
6. Click **Pause** to mute the microphone and local playback. **Resume** restores
   them. The remote service may still process audio and send events while paused.
7. Click **Stop** to close the peer, data channel, and microphone tracks. Leaving
   the page also cleans up recording and realtime media.

Realtime settings apply on the next Start and stay locked during the session.
Each Start creates a new peer and clears the displayed transcript. Saving to
ChatGPT history does not make a conversation resumable through this client.
The local key stays in the page input. No upstream token is sent to the browser.

## Signaling and media lifecycle

A non-demo client performs the following sequence:

1. Request an audio stream and create an `RTCPeerConnection`.
2. Add the microphone track with an audio `sendrecv` transceiver.
3. Create a negotiated data channel with an empty label and ID `0`.
4. Create an offer, set its local description, and gather ICE candidates.
5. POST `application/json` containing the real local `sdp` and options to
   `/v1/realtime/sessions`. Send the local bearer key when configured.
6. Apply the returned `sdp` as the remote answer. Retain `session_id` for
   correlation only.
7. Receive audio through the peer's track event and text/events on the data
   channel. Close the peer and tracks on completion, cancellation, or failure.

There are no HTTP pause, resume, stop, delete, conversation-continuation, or event
polling resources. A signaling response does not carry a conversation by itself.
The controller waits for connected transport and an open data channel. Advanced
and Wingman also require successful `startup_telemetry`; Standard does not.
The client startup limit is 90 seconds, separate from upstream HTTP timeouts.
Microphone state is sent when the channel opens and on Pause/Resume.

The JSON API defaults to Advanced, unlike the demo's Standard default. Both
JSON and the library default `history_and_training_disabled` to `true`. The UI
sends the inverse of **Save conversation**. The JSON handler forwards either
history value for all modes, without enforcing the UI's saving restriction.
Availability and live startup behavior require verification for the actual account.

Offers must start with `v=0`, contain audio media, and contain no NUL bytes.
SDP and upstream response bodies are limited to 128 KiB. The JSON body has a
separate limit of `2 * 128 KiB + 4096` bytes. Language tags contain at most 35
ASCII letters, digits, or hyphens. Timezone names contain 1 to 64 ASCII letters,
digits, or `/`, `_`, `+`, `-`. UTC offsets are integer minutes west of UTC,
between -840 and 840.

File transcription and negotiation share the server's concurrency limiter.
Transcription waits for a permit; negotiation returns 429 if none is available.
A negotiation permit covers signaling, not the WebRTC session's lifetime.
Host/Origin checks protect HTML, static assets, and signaling. Transcription
rejects cross-site browser requests but has no exact Host allowlist. Static
assets and health do not require a local key; both POST handlers check the key.
Without a key, non-loopback binds disable browser routes and realtime signaling.
This does not make a keyless non-loopback transcription server safe to expose.

## Events and model identity

The browser accepts `data_message` envelopes with object or JSON-string `data`,
transcript snapshots, compact deltas, and Standard-mode
`streaming_message_update`. It displays visible user/assistant text with no
channel or the `final` channel. Unknown event types stay in the event log.
Parser support is not proof that the live service emits every supported shape.

Modes select private service behavior, not a verified voice model. Standard and
Advanced requests contain legacy `gpt-4o` fields. Wingman reads the account's
current default model and sends it as `backend_model`. Neither request establishes
the served model. The UI displays a bounded `resolved_model_slug` from visible
assistant messages. A reported `gpt-6` does not distinguish Sol from Astra.
There is no verified `gptlive-1.1` selector or established voice price here.

## Hermes file STT and native GPT-Live

File STT compatibility is the narrow multipart seam. An adapter posts audio to
`/v1/audio/transcriptions` and reads `{text}`. A sent `model` value is ignored.
The repository's Python/Node SDK examples demonstrate that request shape, not
completed Hermes or Discord transport integration.

To validate an installed Hermes STT adapter against a candidate revision:

1. Use the same public speech sample bytes for both the old and new servers.
   Record the sample's digest and the candidate source revision.
2. Invoke the actual installed adapter with its existing local authentication
   supplied externally. Record the destination route, multipart fields, response
   status, and transcript without saving keys or auth files.
3. Compare the transcripts and errors. If recording elapsed times, use the same
   method for both calls and distinguish network/upload time from transcription.
4. Keep deployment-specific final checks separate from this adapter comparison.
   Repeat relevant checks if the binary or configuration changes. Keep operational
   addresses and raw evidence outside public docs. A mock or a curl-only probe is
   not this adapter test.

The default-feature release build at source checkpoint `42d19a6` was verified
through the installed, unmodified Hermes STT adapter using identical public JFK
FLAC bytes against the old and new servers. Both returned HTTP 200 with string
transcripts differing only in punctuation. Fourteen local HTTP, security, and
body-limit checks also passed. The legacy service stayed unchanged; the candidate
was stopped cleanly after verification.

This is a file-STT compatibility observation for that binary and fixture. It is
not a performance benchmark, production switch, or proof of native GPT-Live or
Discord integration. Deployment-specific final checks and Dima's manual UAT
remain separate. Browser or agent smoke checks do not replace that manual UAT.

Native Hermes GPT-Live uses an OpenAI project key and a different upstream
contract. Its session request to `POST /v1/live/sessions` nests
`session.delegation.type = "client"` and `transport.type = "webrtc"` with
`transport.sdp`. Delegation uses `session.delegation.created` and
`session.commentary.append`. These are external GPT-Live protocol names, not
Codex Voice routes or events. Codex Voice instead accepts the flat private
`{sdp,mode,voice,...}` offer and returns `{sdp,session_id}`. Changing a base URL
or model name does not translate those protocols. A real bridge would need media
handling, event translation, delegation, and live acceptance.

See the official [Hermes voice documentation](https://hermes-agent.nousresearch.com/docs/user-guide/features/voice-mode)
and [GPT-Live guide](https://developers.openai.com/api/docs/guides/live) for those
separate systems. Neither implies that this server implements them. Discord
voice-channel audio additionally needs its own transport integration.

## Rust API and verification

`codex_voice::audio::TranscriptionClient` uploads files.
`codex_voice::realtime::RealtimeClient::new(CodexAuth, RealtimeClientConfig)`
creates the asynchronous signaling client. `create_session(RealtimeOffer).await`
returns `RealtimeAnswer` with `sdp` and `session_id`. The caller supplies a runtime
and owns WebRTC media. `RealtimeClientConfig::default()` uses 60/15-second
request/connect timeouts, unlike the CLI server's 120/10-second defaults.

Default features enable `server` and `realtime`. Disable both for an audio-only
library. Enable only `server` for an ASR-only HTTP server or only `realtime` for
signaling without Axum. The optional demo needs no JavaScript framework or build.
A default source build uses the pinned Rust 1.98.0 toolchain, CMake,
Clang/libclang, and pkg-config.

The repository verification commands include:

```sh
cargo fmt --check
cargo test --locked --all-targets --all-features
cargo check --locked --no-default-features --all-targets
cargo check --locked --no-default-features --features server --all-targets
cargo check --locked --no-default-features --features realtime --all-targets
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo clippy --locked --all-targets --no-default-features -- -D warnings
node --test tests/*.test.mjs
```

These are instructions, not a claim that this documentation lane ran Rust builds.
Rust tests use local mock upstreams; Node tests simulate browser media. They do
not prove live account access, browser microphone permission, remote audio,
transcript delivery, served model identity, pricing, deployment, or manual UAT.
