# Codex Voice roadmap

Checked items describe implemented behavior or the specific recorded observation.
Release acceptance and deployment remain separate.

- [x] Provide a Rust library, CLI, and HTTP server with shared authentication and
  separate transcription and realtime modules.
- [x] Use loopback port 2600, concurrency 1, a 10 MiB multipart body limit, and
  upstream request and connect timeouts of 120 and 10 seconds by default.
- [x] Expose canonical transcription and private JSON SDP signaling routes with
  local key protection and exact keyed proxy Host authorities.
- [x] Provide a minimal white demo with Transcriptions and Realtime tabs, audio
  preview, newest-first speaker bubbles, and readable Events output.
- [x] Default the demo to Wingman while leaving Save conversation off. Require
  explicit saving for Wingman and Advanced. Allow Standard without saving.
- [x] Verify installed Hermes file-STT compatibility using the same public speech
  fixture against the existing and candidate servers. This observation covers
  transcription only, not native GPT-Live or Discord voice.
- [x] Record the owner's working-demo acceptance and two real upstream captures
  using public sample audio. The historical realtime capture uses Standard with
  saving off. The README now pairs the real transcription with an illustrative
  English Wingman conversation rendered offline.
- [ ] Complete and record the release acceptance checklist on the revision to be
  published. Cover file and microphone transcription, keyed proxy and secure
  origin, explicit saving, real WebRTC audio and events, failed startup,
  Pause/Resume, and Stop cleanup. The owner's working-demo acceptance and the
  historical captures do not by themselves record every check in this list.
- [ ] Confirm manual acceptance before tags, releases, or image publication.
  Docker CI builds on pull requests and main pushes without publishing. Version
  tags and manual Docker workflow runs can publish images. Release CI publishes
  on matching version tags. These triggers do not enforce manual UAT, so the
  release owner must verify the gate before invoking them.
- [ ] Run deployment-specific final checks and authorize service replacement
  separately from source publication.
- [ ] Implement and live-test a Hermes GPT-Live or Discord voice bridge if needed.
  File transcription compatibility does not provide those media transports.
