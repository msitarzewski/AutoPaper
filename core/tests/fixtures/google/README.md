# Google Gemini API response fixtures

**Documented shape, not recorded.** Every file here is hand-written from the shapes in
`docs/research/providers.md` (verified against Google's docs on 2026-10-05). No key was used and no call
was made. Values (IDs, token counts, text, display names) are illustrative; field names and nesting
follow the documentation.

| File | Endpoint | Source in `docs/research/providers.md` |
|---|---|---|
| `generate_content_text.json` | `POST /v1beta/models/{model}:generateContent` (structured text) | §2.3.4 "Response: `candidates[0].content.parts[]` … Text parts may also appear" and §2.4 (the text part holds the JSON). `finishReason` per §2.3.5. `usageMetadata` (`promptTokenCount`, `candidatesTokenCount`, `thoughtsTokenCount`, `totalTokenCount`), `modelVersion` and `responseId` follow the GenerateContentResponse reference it cites [G-GC-REF]; the research doc doesn't print them. |
| `generate_content_image.json` | `POST /v1/models/{model}:generateContent` (image) | §2.3.4 "Response: `candidates[0].content.parts[]` with `inlineData {mimeType, data(base64)}`. Text parts may also appear." `data` is a real 2×1 PNG (72 bytes). |
| `generate_content_prompt_blocked.json` | image or text | §2.3.5 "`promptFeedback.blockReason` is one of `SAFETY \| OTHER \| BLOCKLIST \| PROHIBITED_CONTENT \| IMAGE_SAFETY`, and no candidates are returned." |
| `generate_content_image_safety.json` | image | §2.3.5 "`candidates[].finishReason` is one of `IMAGE_SAFETY \| …`" (no content). |
| `generate_content_no_image.json` | image | §2.3.5 `finishReason` `NO_IMAGE`, with a text part instead of an image. |
| `generate_content_image_other.json` | image | §2.3.5 `finishReason` `IMAGE_OTHER` (not safety-related). |
| `models_list_page1.json`, `models_list_page2.json` | `GET /v1beta/models?pageSize=1000` | §2.2 (fields `name`, `baseModelId`, `version`, `displayName`, `description`, `inputTokenLimit`, `outputTokenLimit`, `supportedGenerationMethods`, `thinking`, `temperature`, `maxTemperature`, `topP`, `topK`; paging with `nextPageToken` / `pageToken`). Model IDs from §2.3.1, §2.4 and §2.5. |
| `error_api_key_invalid.json` | any (HTTP 400) | **Not in the research doc.** Google's standard API error envelope (`error.code`, `error.message`, `error.status`, `error.details[]` with a `google.rpc.ErrorInfo` `reason`), with the "API key not valid" message the provider maps to `InvalidKey`. |

Written inline in the tests instead (their shape is beside the point or not in the research doc): bodies
for the shared status mapping (`net::error_for_status`: 403, 429 with `Retry-After`, 503, a non-key 400),
text responses with `thought: true` parts, `SAFETY` and `MAX_TOKENS` finish reasons, an image response whose
first part is a Pro "thought image" (`thought: true`) with the final image in snake_case `inline_data` /
`mime_type`, and malformed replies.
