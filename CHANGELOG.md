# Changelog

## Unreleased

- Reorganize the Rust library and server into shared auth/server and separate
  audio/realtime domains. Rename the package/binary to `codex-voice`, crate to
  `codex_voice`, and application environment variables to `CODEX_VOICE_*`.
- Default the server to loopback port 2600, concurrency 1, a 10 MiB multipart
  request-body limit, and 120/10-second upstream request/connect timeouts.
- Expose `POST /v1/audio/transcriptions` and private JSON
  `POST /v1/realtime/sessions`. Remove the obsolete unversioned transcription
  alias and the old voice page/config/session routes.
- Replace the old playground with a minimal static Transcriptions/Realtime demo.
  Keep browser calls on the ordinary API and add microphone file transcription.
- Add repeatable exact `--allowed-host` authorities for keyed private proxies.
- Add audio preview and explicit transcription after microphone recording.
- Show newest-first realtime speaker bubbles and upstream model metadata.
- Default the demo to Wingman with Save conversation off. Require explicit
  saving for Wingman and Advanced, while Standard can run without saving.
- Refresh the English README and focused API, configuration, and voice guides.
  Add two real screenshots using public sample speech, with realtime in Standard
  mode and saving off. These observations do not establish complete release UAT
  or deployment.
- Clarify that upstream account usage limits still apply. Official Chat mode
  allowances do not establish private realtime quotas or transcription limits.

## Inherited transcription history

These entries describe ASR ancestry, not Codex Voice releases. The MIT copyright
and permission notice remain unchanged.

### 0.1.2, 2026-05-05

- Add a REST upload size limit and 413 responses.
- Reload file auth per transcription and configure upstream request/connect
  timeouts.

### 0.1.1, 2026-05-05

- Add upstream build, test, lint, and packaging automation.

### 0.1.0, 2026-05-05

- Add CLI/library transcription with local Codex ChatGPT auth, a multipart HTTP
  wrapper, local key protection, configurable concurrency, and external SILK decoding.
