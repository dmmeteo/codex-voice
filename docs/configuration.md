# Configuration reference

## Authentication

Codex Voice uses an existing Codex ChatGPT subscription login to access private
ChatGPT transcription and voice services. The optional browser demo is a test
client for the server. It does not provide account access or a separate quota.

The Codex ChatGPT login authenticates upstream requests. The local server key
protects the two POST endpoints. They are separate credentials.

`--bearer` takes precedence over `CODEX_VOICE_BEARER`. Either bearer source takes
precedence over file auth. Otherwise `--auth-file` selects the file, followed by
`$CODEX_HOME/auth.json`, then `~/.codex/auth.json`. The file must contain a Codex
ChatGPT login with an access token. The server reads it at startup and reloads
it for each transcription or negotiation. It does not refresh tokens.

`serve` requires `--api-key` or `CODEX_VOICE_API_KEY`, unless `--no-api-key` is
explicitly set. HTTP clients send `Authorization: Bearer <local-key>`.
`--no-api-key` disables local key checks even if a key is supplied elsewhere.
It is intended only for trusted loopback use.

The browser keeps the local key in its input field. It receives no upstream
token, private status ticket, auth-file content, or account identifier.
Keep upstream credentials outside the browser, checkout, and shared examples.

## CLI options

Explicit flags override the corresponding environment values. Global options
can precede the subcommand. The following options apply to upstream requests:

| Option | Environment variable | Default or purpose |
| --- | --- | --- |
| `--auth-file` | `CODEX_HOME` selects the default directory | Codex login file described above |
| `--bearer` | `CODEX_VOICE_BEARER` | Explicit upstream token instead of file auth |
| `--account-id` | None | Account override for explicit bearer auth |
| `--proxy` | `CODEX_VOICE_PROXY`, then standard proxy variables | macOS system HTTPS proxy if available |
| `--request-timeout-seconds` | `CODEX_VOICE_REQUEST_TIMEOUT_SECONDS` | `120`, positive integer seconds |
| `--connect-timeout-seconds` | `CODEX_VOICE_CONNECT_TIMEOUT_SECONDS` | `10`, positive integer seconds |
| `--language` | None | Optional transcription language hint |
| `--endpoint` | None | Upstream transcription URL, not a realtime endpoint override |
| `--content-type` | None | CLI upload MIME type, otherwise inferred from filename |
| `--filename` | None | CLI multipart filename override |
| `--json` | None | CLI JSON output instead of plain text |
| `--silk-decoder` | `CODEX_VOICE_SILK_DECODER` | External `rust-silk` decoder executable |
| `--silk-sample-rate` | None | `24000` Hz for SILK decoding |
| `--no-silk-decode` | None | Upload SILK unchanged instead of decoding |

Proxy resolution uses the first nonempty value in this order: `--proxy`,
`CODEX_VOICE_PROXY`, `HTTPS_PROXY`, `https_proxy`, `ALL_PROXY`, `all_proxy`, then
the macOS system HTTPS proxy. `--language` does not configure realtime language.
Decoded SILK uploads use WAV metadata.

The `serve` subcommand has these options:

| Option | Environment variable | Default |
| --- | --- | --- |
| `--host` | None | `127.0.0.1` |
| `--port` | None | `2600` |
| `--api-key` | `CODEX_VOICE_API_KEY` | Required unless explicitly disabled |
| `--no-api-key` | None | Off |
| `--concurrency` | None | `1`, must be positive |
| `--max-upload-mb` | `CODEX_VOICE_MAX_UPLOAD_MB` | `10` MiB for the entire multipart body |
| `--allowed-host HOST[:PORT]` | None | No additional authorities, repeatable, requires a local key |

`CODEX_VOICE_BASE_URL` configures the example clients, not the server.
Its default is `http://127.0.0.1:2600/v1`.

## Browser access and reverse proxies

HTML, static assets, and realtime signaling check the exact Host authority.
They also reject a supplied Origin whose authority differs from Host.
Transcription checks same-origin browser requests without an exact Host allowlist.
Health and static assets do not require a local key. Both POST handlers check it.
Forwarded headers do not authorize a request.

Additional proxy authorities require a key. With `CODEX_VOICE_API_KEY` supplied:

```sh
codex-voice serve --allowed-host voice.example --allowed-host voice.example:8443
```

These are placeholder authorities. Entries contain no scheme, path, user info,
or wildcard. The proxy must preserve the allowlisted Host and matching Origin.
The allowlist does not configure TLS or proxy access control. Use the same HTTPS
origin for the demo and API when connecting from a remote browser.

A keyless non-loopback bind disables browser routes and realtime signaling,
but still permits transcription requests. Keep keyless servers on loopback.

## Docker Compose

`deploy/compose.yml` publishes the container port only on host loopback.
It requires `CODEX_VOICE_API_KEY` and `CODEX_VOICE_AUTH_FILE`. The latter names
the host login file mounted read-only inside the container.

Compose uses `CODEX_VOICE_PORT` for the published host port and
`CODEX_VOICE_CONCURRENCY` to pass `--concurrency`. These are Compose settings,
not environment variables read by the CLI. Upload limits, timeouts, and the
optional upstream proxy use the variables in the tables above.

The Compose image reference does not establish that a particular image has
passed manual acceptance. Publication and deployment remain separate decisions.
