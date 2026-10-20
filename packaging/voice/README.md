# Codex Voice for macOS

This local package runs the `codex-voice` server and its web client. It uses
your existing Codex Desktop login. The package contains no credentials.

1. Extract the ZIP and keep its five files together.
2. In Terminal, run `./start.command` from the extracted folder. To use another
   port, run `./start.command --port 2601`.
3. Open `http://127.0.0.1:2600/`, or use the port you selected. Allow
   microphone access when the browser asks.
4. Stop the server with Control-C in Terminal.

`start.command` uses the CLI's loopback address and port 2600 defaults. It passes
`--no-api-key`. For a local API key, run `./codex-voice serve --api-key YOUR_KEY`
directly.
The server reads Codex authentication from `$CODEX_HOME/auth.json` or
`~/.codex/auth.json` when you start a session. Do not place an auth file in this
folder. The browser needs network access to ChatGPT for voice sessions.

Read `VOICE.md` for mode settings, session behavior, API fields, and limits.
Codex Voice is based on https://github.com/Wangnov/codex-asr. `LICENSE-MIT`
contains the upstream license and attribution.
