# Working on Codex Voice

Read this file at entry and after context loss. Read README.md for current usage.
Read docs/api.md and docs/configuration.md for HTTP schemas and configuration. Read docs/voice.md for media lifecycle and
integration boundaries. Read ROADMAP.md when changing unfinished accepted work.

## Accepted rules

- Present Codex Voice as an unofficial server that reproduces reverse-engineered
  ChatGPT app flows for transcription and voice conversations using an existing
  Codex ChatGPT subscription login. Feature access and upstream limits still apply.
  The Rust library and CLI are additional entry points. The optional tiny static
  JavaScript demo lets self-hosters test their server and account access without
  writing a client. WebRTC is a transport detail, not the product's purpose.
  The demo uses the server's documented HTTP API. Add no demo configuration,
  session, or event application endpoints. Static assets are allowed.
- Keep `POST /v1/audio/transcriptions` canonical. The historical unversioned
  alias was removed after installed-client verification. Do not restore it
  without a verified consumer requirement.
- `POST /v1/realtime/sessions` is private ChatGPT JSON SDP signaling. It is not
  public OpenAI Realtime or native GPT-Live compatibility. `session_id` is for
  correlation. WebRTC owns media/events; the client owns Pause and Stop.
- Default to loopback `127.0.0.1:2600` with configurable host/port. Keep the
  default concurrency 1, multipart body limit 10 MiB, and request/connect
  timeouts 120/10 seconds. The body limit includes multipart overhead.
- Require a local key unless the operator explicitly chooses trusted-loopback
  `--no-api-key`. Additional exact `--allowed-host` authorities require a key.
  Preserve Host/Origin checks and do not trust forwarded headers as authorization.
- The demo defaults to Wingman with Save conversation off. Wingman and Advanced
  require explicit saving in the UI. Standard can run without it. The JSON API
  defaults to Advanced and does not enforce the UI saving requirement.
- Keep saving to ChatGPT history an explicit user choice. Keep upstream tokens,
  private tickets, account identifiers, and auth contents out of browser data,
  public docs, examples, and reports.
- Distinguish implemented, offline-tested, live-tested, and deployed claims.
  Model names and request fields do not establish the served backend. Do not
  claim a `gptlive-1.1` selector, voice price, or completed Hermes/Discord voice
  bridge without actual evidence.
- Preserve LICENSE-MIT exactly, including the inherited contributors
  copyright and permission notice. Upstream artifacts are not this project's
  distribution. Published metadata does not prove a release passed manual UAT.

## Paths and ownership

src/main.rs owns CLI/start, src/lib.rs the public Rust API, src/auth.rs shared
auth, and src/server.rs HTTP composition/shared limits. src/audio/ and
src/realtime/ each contain mod.rs, client.rs, and routes.rs. web/index.html,
index.js, transcriptions.js, realtime.js, and styles.css form the static demo.
The package/binary name is `codex-voice`, the crate is `codex_voice`, and
application environment variables use `CODEX_VOICE_*`.

During concurrent work, one documentation owner handles README.md, AGENTS.md,
ROADMAP.md, CHANGELOG.md, and docs/. The implementation/operations owner handles
source, web, tests, examples, Cargo, CI, deployment, and packaging. Stay inside
the assigned lane. Do not change a live service, auth/config, workflow, or release
state as a side effect of a documentation edit.

Keep README as the user/API entry, this file as the working boundaries, and
ROADMAP as the single feature checklist. Keep plans, reports, generated research,
and test output outside the repository. Do not add canon directories, mirrored
specifications, implementation inventories, or nonexistent media references.

## Verification

For documentation, compare defaults, names, route inventory, formats, and
examples with source. Resolve local links, preserve legal notices, and review
the full diff. For implementation, run the affected Cargo/Node checks. Node
browser fixtures and Rust mock upstreams do not prove live voice behavior.

Before release, reconcile ROADMAP's manual UAT/publication gate with actual CI
triggers. Do not publish or deploy from an assignment that grants only local
commits. A green documentation check is not final API/integration acceptance.
