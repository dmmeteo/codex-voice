#!/usr/bin/env node
import fs from "node:fs";
import OpenAI from "openai";

const audio = process.argv[2] ?? "audio.wav";
const client = new OpenAI({
  apiKey: process.env.CODEX_VOICE_API_KEY ?? "local_dev_key",
  baseURL: process.env.CODEX_VOICE_BASE_URL ?? "http://127.0.0.1:2600/v1",
});

const result = await client.audio.transcriptions.create({
  model: "whisper-1",
  file: fs.createReadStream(audio),
  response_format: "json",
});

console.log(result.text);
