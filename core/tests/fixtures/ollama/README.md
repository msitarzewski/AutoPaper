# Ollama fixtures

Recorded on 2026-10-05 from the Ollama running on the developer's Mac (`GET /api/version` →
`0.34.2`, `http://127.0.0.1:11434`). Bodies are as the server sent them, re-indented; nothing was
edited except where noted. Used by `core/src/providers/ollama.rs`.

| File | Request | Notes |
|---|---|---|
| `tags.json` | `GET /api/tags` | Trimmed to 3 of the 13 installed models (`ornith:latest`, `qwen2.5vl:7b`, `nomic-embed-text:latest`), order kept. Recent releases list `capabilities`; the embedding model has only `["embedding"]`. |
| `chat-request.json` | `POST /api/chat` | The request that produced `chat-response.json`: concept schema in `format`, `stream: false`, `think: false`, `options.temperature`, and `keep_alive: 0` (added only for the recording, to unload the model afterwards; the provider doesn't send it). |
| `chat-response.json` | ↑ | `ornith:latest` (Qwen 3.5 9B, a thinking model) with `think: false`: no `thinking` field; the JSON arrives as a string in `message.content`; `prompt_eval_count` / `eval_count` are the token counts. |
| `chat-response-thinking.json` | same request without `think` | The same model thinking by default: `message.thinking` holds the reasoning (ignored), `message.content` the JSON. 2186 output tokens against 977 without thinking. |
| `error-model-not-found.json` | `POST /api/chat` with `"model": "no-such-model"` | HTTP **404**, body `{"error": "model 'no-such-model' not found"}`. |

Also verified live (not stored): `think: false` sent to a model without thinking (`qwen2.5vl:7b`) is
accepted. Ollama's image generation is gone (`docs/research/providers.md` §3.1), so there are no image
fixtures.
