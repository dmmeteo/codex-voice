# Codex Voice for macOS

This local package runs the unofficial `codex-voice` server for transcription
and voice conversations through reverse-engineered private ChatGPT services.
It uses your existing Codex Desktop ChatGPT login and the feature access of
that account. The package contains no credentials.

The included browser demo is optional. Open it to test your local server and
account access without building a client. You can also use the server's HTTP
API directly. The server is the product; the browser page is a test client.

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

Read `VOICE.md` for mode settings, media lifecycle, and usage limits.
`LICENSE-MIT` contains the inherited copyright and full permission notice.

Realtime defaults to Wingman with **Save conversation** off. Enable saving
explicitly to start Wingman or Advanced, or choose Standard to keep saving off.
For file transcription, preview the upload or recording, then click **Transcribe**.
The package is a local source build, not evidence of a published or UAT-approved
release.
