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
- Reconcile English documentation with source checkpoint 42d19a6, retaining
  private-service, native GPT-Live, model-identity, and live verification limits.
  These entries describe implementation, not completed manual UAT or deployment.

## Inherited codex-asr history

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
