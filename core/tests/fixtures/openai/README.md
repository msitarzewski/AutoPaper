# OpenAI response fixtures

**Documented shape, not recorded.** Every file here is hand-written from the shapes in
`docs/research/providers.md` (verified against OpenAI's docs on 2026-10-05). No key was used and no
call was made. Values (IDs, timestamps, token counts, text) are illustrative; field names and nesting
follow the documentation.

| File | Endpoint | Source in `docs/research/providers.md` |
|---|---|---|
| `responses_compose.json` | `POST /v1/responses` | §1.3 "Response (abridged …)" [OAI-RESP-REF]. The research sample is abridged; `id`, `created_at`, `error`, `incomplete_details` and `usage` (`input_tokens`, `output_tokens`, `total_tokens`, `*_details`) follow the Responses API reference it cites, not a sample printed in the research doc. |
| `responses_refusal.json` | `POST /v1/responses` | §1.3 "Refusal: the content item instead has `{"type": "refusal", "refusal": "..."}`" |
| `responses_incomplete.json` | `POST /v1/responses` | §1.3 "Truncation: `status: "incomplete"` with `incomplete_details.reason: "max_output_tokens"`" |
| `images_generations.json` | `POST /v1/images/generations` | §1.4.2 "Response". `output_format` is `jpeg` because the provider requests jpeg; `b64_json` is a real 8×8 JPEG (287 bytes). |
| `images_moderation_blocked.json` | `POST /v1/images/generations` (error body) | §1.4.7 "Safety and refusal shape", verbatim. The HTTP status is unconfirmed (§6 item 1; probably 400), so tests send it as 400. |
| `models_list.json` | `GET /v1/models` | §1.2 (fields `id`, `object`, `created`, `owned_by`, `shutdown_date`). The ID mix (text, image, embedding, audio, moderation, legacy, a fine-tune, a duplicate) exercises the picker's filtering. |

Written inline in the tests instead (their shape is beside the point or not in the research doc): bodies
for the shared status mapping (`net::error_for_status`: 401, 429 with `Retry-After`, 503, a 400 that echoes
the key), the out-of-credit 429 (`insufficient_quota`; that code comes from OpenAI's error-code guide, not
the research doc), a response with a `reasoning` item before the message (what reasoning models return),
an `incomplete` / `content_filter` response, and malformed replies.
