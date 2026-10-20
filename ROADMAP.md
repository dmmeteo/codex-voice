# Codex Voice roadmap

Checked items describe implemented source structure and client behavior, not a
released or deployed service. Live acceptance remains separate.

- [x] Reorganize the Rust API, shared auth/server, and audio/realtime domains with
  CLI/start in main.rs and public API in lib.rs.
- [x] Use Codex Voice package, binary, crate, and environment names with default
  loopback port 2600, concurrency 1, 10 MiB multipart body limit, and 120/10-second
  upstream request/connect timeouts.
- [x] Expose canonical transcription and private JSON realtime session routes.
  Remove the verified obsolete alias and old voice/config routes. Add exact
  keyed private proxy Host authorities with `--allowed-host`.
- [x] Provide a minimal white static demo with Transcriptions and Realtime views.
  Keep options left, transcript right, and Transcript/Events output for Realtime.
  Use only the ordinary public API and static assets.
- [ ] Complete manual UAT on the integrated revision, including file/microphone
  transcription, keyed proxy and secure origin, explicit history saving, real
  WebRTC audio/events, failed startup, Pause/Resume, and Stop cleanup.
- [x] Verify the default-feature release build at source checkpoint `42d19a6`
  through the installed, unmodified Hermes STT adapter with the same public JFK
  FLAC fixture as the old server. Both returned HTTP 200 with string transcripts
  differing only in punctuation. This establishes file-STT compatibility only.
- [ ] Gate stable images, tags, and releases on Dima's manual UAT. Allow
  publication of cleaned main history only after verified backup and
  publication-gate repair under its separate authorization. Browser or agent
  smoke checks do not replace manual UAT.
- [ ] Run deployment-specific final checks and accept service replacement under
  separate deployment authority. Preserve the existing live service until the
  authorized migration.
- [ ] Define and verify a real Hermes/Discord voice media integration. File STT
  compatibility and private ChatGPT SDP signaling do not implement native
  GPT-Live delegation or Discord audio transport.
- [ ] Record the Screenshot Studio GIF after integrated manual UAT. Link only a
  real authorized artifact.
