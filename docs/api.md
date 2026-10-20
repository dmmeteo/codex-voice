# HTTP API reference

`server` enables the following routes. Session signaling additionally requires
`realtime`. The demo and static assets are compiled into the server binary.

| Method and path | Request | Response and access |
| --- | --- | --- |
| `GET /healthz` | No body | `{"status":"ok"}`, no key check. Does not probe upstream readiness. |
| `POST /v1/audio/transcriptions` | Multipart audio upload | Transcript, local key when configured. Cross-site browser requests are rejected. |
| `POST /v1/realtime/sessions` | JSON audio SDP offer and options | JSON answer and correlation ID, local key when configured, Host/Origin checks. |
| `GET /` | No body | Static HTML demo, Host/Origin checks, no key check. |
| `GET /index.js`, `GET /transcriptions.js`, `GET /realtime.js`, `GET /styles.css` | No body | Static assets with the same checks as the HTML. |

There are no demo configuration, event polling, Swagger, or HTTP pause/stop
endpoints. WebRTC carries ongoing realtime media and events.

## Transcribe an audio file

`file` is required and nonempty. `language` is a hint forwarded upstream and
falls back to the server's `--language` value when omitted. `response_format`
defaults to `json`, returning `{"text":"..."}`. `text` returns plain UTF-8 text.
`verbose_json` returns `task: "transcribe"`, the requested `language`, `duration: null`, `text`,
and `segments: []`. Timestamps, `srt`, and `vtt` are unsupported.

`model`, `prompt`, `temperature`, timestamp granularity, and other unrecognized
multipart fields are ignored. A `model` value such as the examples' `whisper-1`
is for client request shape only. It does not select the actual backend.
The 10 MiB default covers the whole multipart body, including framing and fields.
It does not guarantee room for a full 10 MiB audio file plus overhead.

Given a local `recording.wav` and a local key supplied in the client environment:

```sh
curl --fail-with-body http://127.0.0.1:2600/v1/audio/transcriptions \
	-H "Authorization: Bearer $CODEX_VOICE_API_KEY" \
	-F 'file=@recording.wav;type=audio/wav' \
	-F language=en -F response_format=json
```

Omit Authorization only for an explicitly keyless server. A successful response
has this shape:

```json
{"text":"Recognized speech."}
```

The [curl](../examples/curl.sh), [Python](../examples/python_openai_sdk.py), and
[Node](../examples/node_openai_sdk.mjs) examples target this file-upload contract.
They use `CODEX_VOICE_BASE_URL` with `http://127.0.0.1:2600/v1` as the default,
and `CODEX_VOICE_API_KEY`. Install their client dependencies separately.

## Negotiate private realtime voice

`POST /v1/realtime/sessions` accepts `application/json` with these fields:

| JSON field | Type and default |
| --- | --- |
| `sdp` | Required complete WebRTC audio offer, at most 128 KiB. |
| `mode` | `standard`, `advanced`, or `wingman`. API default `advanced`. |
| `voice` | Identifier of 1 to 64 ASCII letters, digits, underscores, or hyphens. Default `cove`. |
| `language` | Optional tag of 1 to 35 ASCII letters, digits, or hyphens, such as `en-US`. Omitted or `null` means automatic detection. |
| `enable_message_streaming` | Boolean, default `true`. Requests user and assistant text events. |
| `history_and_training_disabled` | Boolean, default `true`. `false` permits saving to ChatGPT history. |
| `timezone` | 1 to 64 ASCII letters, digits, or `/`, `_`, `+`, `-`. Default `UTC`. |
| `timezone_offset_minutes` | Integer minutes west of UTC, from -840 to 840. Default `0`, matching JavaScript `getTimezoneOffset()`. |

Unknown fields are rejected. SDP must begin with `v=0` followed by a newline,
contain an `m=audio ` section, and contain no NUL bytes. The JSON body limit is
266,240 bytes. The upstream response limit is 128 KiB.

The API forwards either history setting for every mode. The demo additionally
requires **Save conversation** for Advanced and Wingman.

Create `offer.json` with your peer's complete SDP
in place of the placeholder:

```json
{"sdp":"<your peer's complete SDP>","mode":"standard","history_and_training_disabled":true}
```

```sh
curl --fail-with-body http://127.0.0.1:2600/v1/realtime/sessions \
	-H "Authorization: Bearer $CODEX_VOICE_API_KEY" \
	-H 'Content-Type: application/json' --data-binary @offer.json
```

The response is `{"sdp":"<answer SDP>","session_id":"<correlation ID>"}`.
The placeholders are not usable SDP. Apply the answer to the originating peer.
Audio and events then flow over WebRTC and its data channel. `session_id` is
correlation, not a server-owned media resource. The client owns microphone,
playback, Pause/Resume, and Stop cleanup. HTTP success alone does not prove a
working conversation. See the [voice guide](voice.md) for lifecycle details.

Application errors use `{"error":{"message":"...","type":"...","param":null,"code":null}}`.
Typical statuses are 400 for malformed options, 401 for a missing/invalid local
key or unavailable server login during negotiation, 403 for browser-origin checks, 413 for body/SDP limits, 429 for busy signaling
or upstream limits, 502 for upstream failure, and 504 for realtime upstream
timeout. Local configuration failures can return 500. Framework rejections may
have different bodies.

Transcription and negotiation share the concurrency limit. Transcription waits
for a permit. Negotiation returns 429 when no permit is available and releases
its permit after signaling, without limiting the lifetime of the media session.

The [configuration reference](configuration.md) describes authentication and
browser access checks. The [voice guide](voice.md) describes WebRTC lifecycle
and event handling.
