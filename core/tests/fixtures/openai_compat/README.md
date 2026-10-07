# OpenAI-compatible fixtures

Used by `core/src/providers/openai_compat.rs`. Sources differ per file. Recorded files are as the
server sent them, re-indented.

| File | Source | Notes |
|---|---|---|
| `chat-completions-request.json` | Recorded 2026-10-05 | The request sent to Ollama 0.34.2's OpenAI-compatible endpoint (`POST http://127.0.0.1:11434/v1/chat/completions`): `response_format: {type: "json_schema", json_schema: {name, strict: true, schema}}`. |
| `chat-completions-response.json` | Recorded, ↑ | `qwen2.5vl:7b`. The JSON arrives as a string in `choices[0].message.content`; `usage.prompt_tokens` / `completion_tokens`. |
| `models.json` | Recorded, `GET /v1/models` on Ollama 0.34.2 | Trimmed to 3 of 13 entries, order kept. |
| `error-model-not-found.json` | Recorded, `POST /v1/chat/completions` with an unknown model | HTTP **404**, OpenAI-style `error.message`. |
| `error-lmstudio-token-required.json` | Recorded, `GET http://127.0.0.1:1234/v1/models` on LM Studio with API tokens required and none sent | HTTP **401**, `error.code: "invalid_api_key"`. |
| `images-b64-response.json` | **Written by hand** in the documented shape | `{"created", "data": [{"b64_json"}]}` (`docs/research/providers.md` §1.4.2, sd-server §3.3). No OpenAI-compatible image server was running to record from. `b64_json` is `tiny.png`. |
| `images-url-response.json` | **Written by hand** in the documented shape | LocalAI-style `data[].url` on the server's own host (§3.3; its default is unverified, §6 item 13). |
| `tiny.png` | Generated | A valid 4 × 2 RGB PNG, the image bytes used throughout. |

Ollama's `/v1/images/generations` answers HTTP 404 `404 page not found` (verified live; image
generation was removed from Ollama in 2026-07).
