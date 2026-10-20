#!/usr/bin/env python3
import os
import sys
from pathlib import Path

import httpx
from openai import OpenAI


def main() -> None:
    audio = Path(sys.argv[1] if len(sys.argv) > 1 else "audio.wav")
    client = OpenAI(
        api_key=os.environ.get("CODEX_VOICE_API_KEY", "local_dev_key"),
        base_url=os.environ.get("CODEX_VOICE_BASE_URL", "http://127.0.0.1:2600/v1"),
        http_client=httpx.Client(trust_env=False),
    )
    with audio.open("rb") as file:
        result = client.audio.transcriptions.create(
            model="whisper-1",
            file=file,
            response_format="json",
        )
    print(result.text)


if __name__ == "__main__":
    main()
