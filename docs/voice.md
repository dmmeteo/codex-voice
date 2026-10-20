# Voice client guide

Codex Voice uploads audio files for transcription and negotiates private ChatGPT
voice sessions. The Rust server handles signaling. WebRTC carries ongoing audio
and events directly between the client and the upstream service.

This guide also ships as `VOICE.md` in the macOS package. The package README
explains the launcher. The repository's `docs/api.md` and
`docs/configuration.md` contain the complete HTTP and configuration references.

## Use the demo

1. Start the server and open `http://127.0.0.1:2600/`. Enter the local server key
   when configured. Remote microphone access requires trusted HTTPS.
2. In **Transcriptions**, choose **Upload file** or **Microphone**. For a recording,
   click **Record**, allow microphone access, then click **Stop**. Preview the
   resulting clip and click **Transcribe**. During upload, **Cancel** aborts the
   request. The transcript appears on the right.
3. In **Realtime**, choose a mode and voice. Wingman and Cove are the defaults.
   **Save conversation** starts off. Wingman and Advanced require you to enable
   saving explicitly. To keep saving off, choose Standard. **Transcripts** starts
   on and requests text independently of saving.
4. Click **Start**, allow microphone access, and wait for **Active**. A successful
   HTTP response alone does not establish readiness. Settings stay locked until
   the session ends.
5. Read **Transcript** for newest-first speaker bubbles. **Events** shows
   timestamped connection and service events, with formatted JSON details.
   **Clear** clears the event list without clearing the transcript.
6. Click **Pause** to mute microphone input and local playback. **Resume** restores
   both. The upstream service may still process audio and send events while paused.
7. Click **Stop** to close the peer, data channel, and microphone tracks.
   Leaving the page also cleans up transcription and realtime media.

Each Start creates a new peer and clears the displayed transcript. Saving to
ChatGPT history does not make a conversation resumable through this client.
The local key stays in the page input. No upstream credential goes to the browser.

## Connect another WebRTC client

1. Request an audio stream and create an `RTCPeerConnection`.
2. Add the microphone track with an audio `sendrecv` transceiver.
3. Create a negotiated data channel with an empty label and ID `0`.
4. Create an offer, set its local description, and gather ICE candidates.
5. POST `application/json` with the complete local `sdp` and options to
   `/v1/realtime/sessions`. Include the local bearer key when configured.
6. Apply the returned `sdp` as the remote answer. Keep `session_id` for
   correlation only.
7. Receive audio through the peer's track event and events on the data channel.
   Close the peer and tracks on completion, cancellation, or failure.

There are no HTTP pause, resume, stop, delete, conversation-continuation, or event
polling resources. The client owns media and cleanup.

The demo waits for connected transport and an open data channel. Advanced and
Wingman also require successful `startup_telemetry`. Standard does not.
The client startup limit is 90 seconds, separate from upstream HTTP timeouts.
The client sends microphone state when the channel opens and on Pause or Resume.

The JSON API defaults to Advanced, while the demo defaults to Wingman.
Both JSON and the Rust library default `history_and_training_disabled` to `true`.
The demo sends the inverse of **Save conversation**. The JSON handler forwards
either history value for every mode without enforcing the demo's saving restriction.
Mode availability and startup behavior depend on the account and upstream service.

## Read events and model metadata

The browser accepts `data_message` envelopes with object or JSON-string `data`,
transcript snapshots, compact deltas, and Standard-mode
`streaming_message_update`. It displays visible user and assistant text with no
channel or the `final` channel. Unknown event types stay in the event log.
Parser support does not prove that the live service emits every supported shape.
Upstream markup, including Entity tags, is rendered as text.

Modes select private service behavior. Standard and Advanced send legacy
`gpt-4o` request fields. Wingman reads the account's current default model and
sends it as `backend_model`. These request fields do not establish the served
model. **Reported model** displays a bounded `resolved_model_slug` from visible
assistant messages. A reported `gpt-6` does not distinguish Sol from Astra.
There is no verified `gptlive-1.1` selector or established voice price here.

## Understand upstream usage limits

Codex Voice does not bypass upstream account limits. Voice availability depends
on your account plan, the feature and mode in use, and upstream policy.

As of October 20, 2026, the official
[ChatGPT Voice documentation](https://help.openai.com/en/articles/20001274-chatgpt-voice)
measures Live usage in Chat mode over a rolling 24-hour period. Its Voice in Chat
table lists 15 hours with GPT-Live-1 for the $100/month Pro plan. This is an
allowance over the preceding 24 hours, not a reset at midnight. The article
also distinguishes desktop voice pricing and limits.

That Chat mode table does not establish which quota applies to Codex Voice's
private Standard, Advanced, or Wingman transport. The 15-hour example is not a
measured Codex Voice allowance. A model's conversational answer cannot establish
remaining quota, fallback behavior, or the identity of the served model.

File transcription and realtime voice use different requests. Voice allowances
do not establish transcription allowances, and successful transcription does
not prove that realtime quota remains. This project reports no remaining quota
and makes no unrestricted or unlimited transcription claim.

## Integrate file transcription

An STT adapter can post multipart audio to `/v1/audio/transcriptions` and read
`{"text":"..."}`. A sent `model` value is ignored. The repository's Python and
Node SDK examples demonstrate this request format.

Native Hermes GPT-Live uses a different signaling and delegation contract.
Changing a base URL or model name does not translate that protocol. A bridge
needs media handling, event translation, delegation, and live acceptance.
Discord voice channels also need their own audio transport integration.
Neither integration is implemented by file STT compatibility or JSON SDP signaling.

To verify an installed STT adapter, compare the same public audio bytes against
the existing and candidate servers through the actual adapter. Record fixture
digests, revisions, statuses, and transcripts outside the repository. Keep keys
and auth contents out of the evidence. Mock or curl-only checks do not establish
installed-adapter compatibility.

## Use the Rust library

`codex_voice::audio::TranscriptionClient` uploads files.
`codex_voice::realtime::RealtimeClient::new(CodexAuth, RealtimeClientConfig)`
creates the asynchronous signaling client. `create_session(RealtimeOffer).await`
returns `RealtimeAnswer` with `sdp` and `session_id`. The caller supplies an async
runtime and owns WebRTC media. `RealtimeClientConfig::default()` uses request and
connect timeouts of 60 and 15 seconds, unlike the CLI server's 120 and 10 seconds.

Default features enable `server` and `realtime`. Disable both for an audio-only
library. Enable only `server` for a transcription HTTP server or only `realtime`
for signaling without Axum. The static demo needs no JavaScript build step.

## Verify a change

Run the affected checks in an environment with the required toolchain:

```sh
cargo fmt --check
cargo test --locked --all-targets --all-features
cargo check --locked --no-default-features --all-targets
cargo check --locked --no-default-features --features server --all-targets
cargo check --locked --no-default-features --features realtime --all-targets
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo clippy --locked --all-targets --no-default-features -- -D warnings
node --test --test-concurrency=1 tests/*.test.mjs
```

Rust mock upstreams and Node browser fixtures provide offline verification.
They do not prove live microphone permission, audio playback, transcript delivery,
model identity, pricing, deployment, or completed manual acceptance.
The README shows a real file transcription and an illustrative English Wingman
conversation rendered offline in the actual UI. The illustration made no
upstream calls and does not verify API behavior. A prior Standard WebRTC capture
used public sample audio with saving off. That observation does not establish
full acceptance for every mode or browser.
