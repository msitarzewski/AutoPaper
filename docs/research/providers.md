# AutoPaper: AI provider API reference (text, image, embeddings)

Research date: 2026-10-05. Every fact below was checked against the provider's official documentation on that date, unless it sits under **Uncertain / unverified** (section 6). Doc pages were downloaded and searched as raw HTML or Markdown wherever possible, so that a summarizing model could not misreport them. A citation tag such as `[OAI-IMG]` points to the URL list in section 7.

> **The model landscape has moved past mid-2025 assumptions:**
> - OpenAI's current image models are `gpt-image-2.5-flare` and `gpt-image-2.5-sunburst` (released 2026-09-08). DALL·E 2 and 3 were retired on 2026-05-12. The cheap current text model is `gpt-6-luna` (released 2026-09-22).
> - Google has **shut down Imagen**, so the `:predict` endpoint is no longer usable for images. Image generation now goes through "Nano Banana" Gemini models. `gemini-2.5-flash-image` was scheduled to shut down on 2026-10-02 (already past). Google now recommends the **Interactions API** (`/v1beta/interactions`) for new projects. `generateContent` is "legacy" but "remains fully supported".
> - Ollama **removed** its experimental image generation (PR #16615, merged 2026-07-28). Current releases (v0.35.1, v0.40.0-rc3) return HTTP 400 `"image generation models are not currently supported"`.

---

## 0. Recommendations at a glance

| Provider | Default TEXT model (structured JSON) | Default IMAGE model | Max native landscape output | Price per image (standard, image output only) |
|---|---|---|---|---|
| OpenAI | `gpt-6-luna` via Responses API, `text.format: json_schema, strict: true`, `reasoning.effort: "none"` or `"low"`, `store: false`. $0.10 in / $0.50 out per 1M tokens | `gpt-image-2.5-flare` (Sunburst is the "editing precision" sibling at the same token rates) | 3840x2160 (16:9, the documented max). 3840x1648 (~21:9). 3632x2272 (~16:10; this size is computed from the documented rules, not listed). >2560x1440 is "experimental" | 3840x2160: low $0.011, medium $0.026, **high $0.100**, xhigh $0.178, max $0.400. 2560x1440: high $0.055. Computed with OpenAI's own calculator formula (see 1.4.6) |
| Google Gemini (AI Studio key) | `gemini-3.5-flash-lite` ($0.30 / $2.50). Cheaper alternative: `gemini-3.1-flash-lite` ($0.25 / $1.50) | `gemini-3.1-flash-image` (Nano Banana 2). Premium: `gemini-3-pro-image` (Nano Banana Pro). Budget: `gemini-3.1-flash-lite-image` (1K only) | **4K at 16:9 = 5504x3072** (larger than 5K 5120x2880). 21:9 = 6336x2688. 3:2 = 5056x3392. No native 16:10 | Nano Banana 2: 0.5K $0.045, 1K $0.067, 2K $0.101, **4K $0.151**. Pro: 1K/2K $0.134, 4K $0.24. Lite: 1K $0.0336. Batch is half price |
| Ollama (local, :11434) | Any local model via `/api/chat` with `format: <JSON Schema>`, or `/v1/chat/completions` with `response_format: json_schema` | **None.** Image generation was removed in July 2026 | n/a | free (local) |
| LM Studio (local, :1234) | `/v1/chat/completions` with `response_format: json_schema` (llama.cpp grammar / MLX Outlines) | None (no images endpoint) | n/a | free |
| ComfyUI (local, :8188) | n/a | Any checkpoint (SDXL/Flux/etc.) via workflow JSON | Limited only by model and VRAM. Upscale nodes are available | free |
| LocalAI (:8080) / stable-diffusion.cpp `sd-server` (:1234) | (LocalAI also does text) | OpenAI-compatible `POST /v1/images/generations` | model dependent | free |

Notes for building the app:
- **5K/6K displays:** no cloud API natively produces 6K (6016x3384). Gemini 4K 16:9 (5504x3072) covers 5K after a downscale. OpenAI tops out at 3840x2160, so 5K needs a local upscaler (for example ComfyUI `UpscaleModelLoader` + `ImageUpscaleWithModel`, or LocalAI `/v1/images/upscale`). Neither OpenAI nor the Gemini API offers an upscale endpoint.
- **16:10 displays:** OpenAI accepts arbitrary `WxH`, so 16:10 can be requested directly. Gemini has no 16:10 ratio, so generate 3:2 or 16:9 and center-crop.
- **Privacy defaults:** set `store: false` on OpenAI Responses (the default is `true`, with ≥30 days retention). Set `store: false` on Gemini Interactions (by default interactions are stored 55 days on paid tier and 1 day on free tier). Gemini **free/unpaid** usage may be used for training and read by human reviewers. AutoPaper should tell users this.

---

## 1. OpenAI

### 1.1 Basics
- Base URL `https://api.openai.com/v1`. Auth header `Authorization: Bearer $OPENAI_API_KEY`. JSON body with `Content-Type: application/json` [OAI-IMG-REF][OAI-SO].
- Docs moved from `platform.openai.com/docs/...` to `developers.openai.com/api/docs/...` (301 redirect).

### 1.2 Model picker: `GET /v1/models` [OAI-MODELS-LIST]
```bash
curl https://api.openai.com/v1/models -H "Authorization: Bearer $OPENAI_API_KEY"
```
```json
{ "object": "list",
  "data": [
    { "id": "model-id-0", "object": "model", "created": 1686935002, "owned_by": "organization-owner", "shutdown_date": null },
    { "id": "model-id-2", "object": "model", "created": 1686935002, "owned_by": "openai", "shutdown_date": "2026-10-23" } ] }
```
- Fields: `id`, `created`, `object`, `owned_by`, `shutdown_date` (nullable; this field is new). There is **no capability flag**, so the app must classify IDs itself. Suggested prefixes: `gpt-image-*` and `chatgpt-image-latest` for images, `text-embedding-*` for embeddings, everything else is a text candidate. `shutdown_date` can be used to warn about deprecated models.

### 1.3 TEXT: structured output

**Current models and prices** (per 1M tokens, Standard tier; Batch/Flex are 50%) [OAI-PRICING][OAI-LUNA]:

| Model | Input | Cached | Output | Notes |
|---|---|---|---|---|
| `gpt-6-luna` (**recommended**) | $0.10 | $0.01 | $0.50 | "most efficient model for focused, high-volume tasks". 1,050,000 ctx, 128,000 max output, knowledge cutoff 2026-05-18. `reasoning.effort`: none, low, medium (default), high, xhigh, max. Structured outputs: Supported. Endpoints: Responses and Chat Completions. Prompts >272K tokens cost 2x input / 1.5x output |
| `gpt-6.1-sol` | $2.00 | $0.10 | $10.00 | does **not** support `none` effort |
| `gpt-6-astra` | $10.00 | $1.00 | $50.00 | flagship |
| `gpt-5.4-nano` | $0.20 | $0.02 | $1.25 | older |
| `gpt-5-nano` | $0.05 | $0.005 | $0.40 | older, cheapest listed |
| `gpt-4o-mini` | $0.15 | $0.075 | $0.60 | legacy |

- `gpt-6-luna` rate limits (RPM / TPM): Free not supported. Tier 1 500 / 500K. Tier 2 5,000 / 2M. Tier 3 5,000 / 4M. Tier 4 10,000 / 10M. Tier 5 30,000 / 180M [OAI-LUNA].
- When reasoning effort is not `none`, remove `temperature`, `top_p`, and `top_logprobs` (Chat Completions: also `logprobs`) [OAI-GPT6]. Use `reasoning.effort` in Responses and `reasoning_effort` in Chat Completions.
- Structured Outputs is available "starting with GPT-4o" [OAI-SO].

**Responses API request** (`POST /v1/responses`) [OAI-SO]. The documented example is adapted to AutoPaper; the field names are exact:
```json
{
  "model": "gpt-6-luna",
  "store": false,
  "reasoning": { "effort": "low" },
  "input": [
    { "role": "system", "content": "You turn wallpaper keywords into a scene concept. Desktop wallpaper, landscape, no text, no people unless asked." },
    { "role": "user", "content": "Keywords: rain, ruins, peaceful, night, blue. Aspect 16:9." }
  ],
  "text": {
    "format": {
      "type": "json_schema",
      "name": "scene_concept",
      "strict": true,
      "schema": {
        "type": "object",
        "properties": {
          "title": { "type": "string" },
          "subject": { "type": "string" },
          "palette": { "type": "array", "items": { "type": "string" } },
          "mood": { "type": "string", "enum": ["calm", "dramatic", "melancholic", "uplifting"] },
          "image_prompt": { "type": "string" },
          "negative_hints": { "type": ["string", "null"] }
        },
        "required": ["title", "subject", "palette", "mood", "image_prompt", "negative_hints"],
        "additionalProperties": false
      }
    }
  }
}
```
Response (abridged; the shape comes from the API reference) [OAI-RESP-REF]:
```json
{ "object": "response", "status": "completed", "model": "gpt-6-luna",
  "output": [ { "type": "message", "role": "assistant", "status": "completed",
    "content": [ { "type": "output_text", "annotations": [], "text": "{\"title\":\"...\",...}" } ] } ],
  "store": false }
```
- **Refusal:** the content item instead has `{"type": "refusal", "refusal": "..."}`. **Truncation:** `status: "incomplete"` with `incomplete_details.reason: "max_output_tokens"` [OAI-SO][OAI-RESP-REF].
- **Chat Completions equivalent** (`POST /v1/chat/completions`): `"response_format": {"type": "json_schema", "json_schema": {"name": "...", "strict": true, "schema": {...}}}`. The result is in `choices[0].message.content`, or a refusal in `choices[0].message.refusal` [OAI-SO].

**Strict schema rules** [OAI-SO] (verbatim limits):
- Root must be an `object` and not `anyOf`. `additionalProperties: false` is required on every object. **All fields must be in `required`**: emulate optional fields with `"type": ["string", "null"]`.
- Supported: string (`pattern`; `format` values date-time, time, date, duration, email, hostname, ipv4, ipv6, uuid), number/integer (`multipleOf`, `maximum`, `exclusiveMaximum`, `minimum`, `exclusiveMinimum`), array (`minItems`, `maxItems`), `enum`, `anyOf`, `$defs`/definitions, recursive `$ref`.
- Not supported: `allOf`, `not`, `dependentRequired`, `dependentSchemas`, `if`/`then`/`else`.
- Size limits: up to 5000 object properties total, 10 levels of nesting, 120,000 characters total across names/enum/const values, and 1000 enum values. A single string enum with more than 250 values is limited to 15,000 chars.
- Output keys come back in schema order. "The first request you make with any schema will have additional latency", while later requests with the same schema do not, so cache the schema.

### 1.4 IMAGE generation

**1.4.1 Models** [OAI-IMG][OAI-IMG-REF][OAI-CHANGELOG]

| Model ID | Snapshot | Notes |
|---|---|---|
| `gpt-image-2.5-flare` | `gpt-image-2.5-flare-2026-09-08` | "Fast, high-quality everyday image generation". **Recommended default** |
| `gpt-image-2.5-sunburst` | `gpt-image-2.5-sunburst-2026-09-08` | "Our most capable model for image generation and editing", for when "editing precision matters most" |
| `gpt-image-2` | `gpt-image-2-2026-04-21` | previous generation. Quality up to `high`, same arbitrary sizes |
| `gpt-image-1.5`, `gpt-image-1`, `gpt-image-1-mini`, `chatgpt-image-latest` | | legacy. Fixed sizes 1024x1024, 1536x1024, 1024x1536 |
| `dall-e-2`, `dall-e-3` | | **retired 2026-05-12** |

Both 2.5 models "use GPT Image 2 token rates" [OAI-CHANGELOG].

**1.4.2 Endpoint and request** (`POST https://api.openai.com/v1/images/generations`) [OAI-IMG-REF]:
```json
{
  "model": "gpt-image-2.5-flare",
  "prompt": "Ancient stone ruins at night in gentle rain, deep blue palette, ... (max 32000 chars)",
  "size": "3840x2160",
  "quality": "high",
  "output_format": "png",
  "background": "opaque",
  "moderation": "auto",
  "n": 1,
  "user": "<stable hashed per-install id, optional>"
}
```
| Param | Values |
|---|---|
| `model` | required, specify explicitly |
| `prompt` | required. "The maximum length is 32000 characters" |
| `size` | `auto`, `1024x1024`, `1536x1024`, `1024x1536`, or **any `WIDTHxHEIGHT`** for gpt-image-2 / 2.5 (rules in 1.4.3) |
| `quality` | `low`, `medium`, `high`, `auto` (default). 2.5 models add **`xhigh`, `max`**. `standard`/`hd` are DALL·E only (deprecated) |
| `output_format` | `png` (default), `jpeg`, `webp`. "Using jpeg is faster than png" [OAI-IMG] |
| `output_compression` | 0–100 (jpeg/webp only), default 100 |
| `background` | `transparent`, `opaque`, `auto` (transparent needs png/webp) |
| `moderation` | `auto` (default) or `low` ("less restrictive filtering") |
| `n` | 1–10 |
| `stream` | bool. `partial_images` 0–3 for progressive previews (SSE events `image_generation.partial_image` and `image_generation.completed`) |
| `response_format` | **deprecated. GPT image models always return base64** (`url` is "Unsupported for GPT image models") |
| `style` | deprecated, unsupported for GPT image models |
| seed | **no seed parameter exists** |
| negative prompt | **no negative-prompt parameter**. Put exclusions in the prompt text |

**Response** [OAI-IMG-REF]:
```json
{ "created": 1713833628,
  "background": "opaque", "output_format": "png", "quality": "high", "size": "3840x2160",
  "data": [ { "b64_json": "..." } ],
  "usage": { "input_tokens": 50, "input_tokens_details": { "text_tokens": 50, "image_tokens": 0 },
             "output_tokens": 3336, "total_tokens": 3386,
             "output_tokens_details": { "image_tokens": 3336, "text_tokens": 0 } } }
```
(`revised_prompt` is "Not returned by GPT image models".)

**1.4.3 Size rules** (gpt-image-2 / 2.5), quoted exactly [OAI-IMG][OAI-IMG-REF]:
> Width and height must be multiples of 16, the aspect ratio must be between 1:3 and 3:1, and neither edge may exceed 3840 pixels. The total pixel count must be between 655,360 and 8,294,400 (4K). Resolutions above `2560x1440` are experimental.
> ...the maximum supported resolution is `3840x2160`.

Desktop sizes that pass these rules:

| Target | Request size | Pixels | Notes |
|---|---|---|---|
| 16:9 max | `3840x2160` | 8,294,400 (= cap) | experimental tier |
| 16:9 safe | `2560x1440` | 3,686,400 | highest non-experimental 16:9 |
| 16:10 max | `3632x2272` (my calculation) | 8,251,904 | 3840x2400 is **invalid** (9.2 MP is over the cap) |
| 16:10 | `2560x1600` | 4,096,000 | experimental (above 2560x1440) |
| ~21:9 max | `3840x1648` (2.33:1) | 6,328,320 | |
| 21:9 (3440x1440 panel) | `3440x1440` | 4,953,600 | 3440 and 1440 are both multiples of 16 |
| 1080p | `1920x1088` | | 1080 is not a multiple of 16, so generate 1088 and crop |

**1.4.4 Latency:** "Complex prompts may take up to 2 minutes to process" [OAI-IMG]. Third-party coverage of the launch says Flare has "50% lower latency than GPT-Image-2" (not stated on the official docs page). Recommended client timeout: ≥180 s, or stream with `partial_images` to show progress.

**1.4.5 Rate limits** (same for flare, sunburst, and gpt-image-2) [OAI-FLARE][OAI-SUNBURST]: Tier 1 100K TPM / **5 IPM**. Tier 2 250K / 20. Tier 3 800K / 50. Tier 4 3M / 150. Tier 5 8M / 250. New Tier-1 users can only make 5 images per minute.

**1.4.6 Pricing.** Image output costs **$30 / 1M tokens** (Batch $15). Text input $5 / 1M (cached $1.25). Image input $8 / 1M [OAI-PRICING][OAI-FLARE]. The pricing page has no per-image table; it points to the calculator in the guide. I pulled the calculator's source (`GptImageTokenCalculator.react.*.js`, rendered with `outputPricePerMillion: 30`). Its formula reproduces OpenAI's published gpt-image-2 table exactly (1024²: $0.006/$0.053/$0.211; 1536x1024: $0.005/$0.041/$0.165), so it is the official method:
```
base = {gpt-image-2: {low:16, medium:48, high:96},
        gpt-image-2.5: {low:16, medium:24, high:48, xhigh:64, max:96}}[model][quality]
long = max(w,h); short = min(w,h)
s = base / (long/short); u = round-half-even(s)
grid = (w>=h) ? base*u : u*base
output_tokens = ceil(grid * (2e6 + w*h) / 4e6)
cost = output_tokens * 30 / 1e6   (USD, excludes text/image input tokens and streamed partials)
```
**gpt-image-2.5-flare / sunburst** (image output only, Standard tier):

| Size | low | medium | high | xhigh | max |
|---|---|---|---|---|---|
| 1024x1024 | $0.0059 | $0.0132 | $0.0527 | $0.0937 | $0.2107 |
| 1536x1024 | $0.0047 | $0.0103 | $0.0412 | $0.0738 | $0.1646 |
| 1920x1088 | $0.0044 | $0.0103 | $0.0398 | $0.0707 | $0.1590 |
| 2560x1440 | $0.0062 | $0.0143 | $0.0553 | $0.0983 | $0.2211 |
| 2560x1600 | $0.0073 | $0.0165 | $0.0659 | $0.1171 | $0.2634 |
| 3440x1440 | $0.0059 | $0.0125 | $0.0501 | $0.0901 | $0.2003 |
| 3840x1648 | $0.0070 | $0.0150 | $0.0630 | $0.1079 | $0.2459 |
| 3632x2272 | $0.0123 | $0.0277 | $0.1107 | $0.1969 | $0.4429 |
| **3840x2160** | $0.0111 | $0.0260 | **$0.1001** | $0.1779 | $0.4003 |

For gpt-image-2 (3 tiers only), 3840x2160 costs low $0.011, medium $0.100, high $0.400 (its medium equals 2.5's high). Add about $0.001–0.003 for prompt text tokens. Legacy per-image prices (1024²/1536x1024): gpt-image-1.5 high $0.133/$0.20, gpt-image-1 high $0.167/$0.25, gpt-image-1-mini high $0.036/$0.052 [OAI-IMG].

**1.4.7 Safety and refusal shape** [OAI-IMG]:
```json
{ "error": { "type": "image_generation_user_error", "code": "moderation_blocked",
  "moderation_details": { "moderation_stage": "input", "categories": ["harassment"] } } }
```
- `moderation_stage`: `input`, `output`, or `unknown`. `categories` holds coarse labels such as harassment, self-harm, sexual, violence.
- "Don't automatically retry these errors without modifying the prompt." Use `error.code` as the stable discriminator. Keep the user-facing message generic.
- Retry 429/5xx with backoff. Honor `Retry-After` when present. Do not auto-retry quota errors (429 "Credit balance exhausted", or spend/usage limit reached) [OAI-ERRORS].

**1.4.8 Upscale:** OpenAI has **no** upscale endpoint. Edits (`/v1/images/edits`) exist but are not upscalers.

### 1.5 Embeddings [OAI-EMB][OAI-PRICING]
| Model | Default dims | `dimensions` param | Max input | Price / 1M tokens |
|---|---|---|---|---|
| `text-embedding-3-small` | 1536 | yes (shortenable) | 8192 | $0.02 |
| `text-embedding-3-large` | 3072 | yes | 8192 | $0.13 |
| `text-embedding-ada-002` | 1536 | no | 8192 | $0.10 |

Endpoint: `POST /v1/embeddings` with `{"model": "text-embedding-3-small", "input": "...", "dimensions": 512}`.

### 1.6 Data retention / privacy (for PRIVACY.md) [OAI-DATA][OAI-RESP-REF]
- "As of March 1, 2023, data sent to the OpenAI API is not used to train or improve OpenAI models (unless you explicitly opt in)."
- Abuse-monitoring logs "may contain certain customer content, such as prompts and responses... retained for up to 30 days" by default. This applies to `/v1/responses`, `/v1/chat/completions`, `/v1/images/generations`, and `/v1/embeddings` (30 days each).
- Application state: `/v1/images/generations` keeps none. `/v1/chat/completions` keeps none (with exceptions). **`/v1/responses` defaults `store: true`, "response data will be stored for at least 30 days"**, so AutoPaper should send `store: false`.
- Zero Data Retention and Modified Abuse Monitoring require OpenAI approval at the org level, which the app cannot control.
- The `user` field "can help OpenAI to monitor and detect abuse". Send a hashed, non-PII per-install ID, or omit it.

---

## 2. Google Gemini API (AI Studio key, not Vertex)

### 2.1 Basics
- Base `https://generativelanguage.googleapis.com`. Header **`x-goog-api-key: $GEMINI_API_KEY`** (in every official curl example) [G-IMG][G-SO].
- **Two API surfaces** [G-INTERACTIONS]:
  - **Interactions API** `POST /v1beta/interactions`. "As of June 2026, it is Generally Available and recommended for all new projects." New features launch only here. Custom `safetySettings`, Batch, and explicit caching are not supported.
  - **generateContent** `POST /v1beta/models/{model}:generateContent`. Image examples now use `/v1/models/{model}:generateContent`. It is "legacy" but "remains fully supported", and it supports Batch and safety settings. **Verified 2026-10-06:** `/v1` serves only the stable image models (4 of them); preview models, `nano-banana-pro-preview` and `gemini-nano-banana-2.1` exist only on `/v1beta` (8), and `/v1` answers 404 for them. AutoPaper lists models from `/v1beta`, so it paints there too.
  - For AutoPaper, both work. generateContent has better-documented safety and finish reasons (2.3.5).
- **Imagen is shut down.** "Imagen models are shut down. Use Nano Banana for image generation." The `models.predict` / `imagen-4.0-*` path is gone, and so is `negativePrompt` (an Imagen-only feature) [G-IMAGEN][G-MODELS].

### 2.2 Model picker: `models.list` [G-API-MODELS]
```bash
curl "https://generativelanguage.googleapis.com/v1beta/models?pageSize=1000" -H "x-goog-api-key: $GEMINI_API_KEY"
```
- Default page size 50, max 1000. Paginate with `pageToken` / `nextPageToken`.
- Model fields: `name` (`models/...`), `baseModelId`, `version`, `displayName`, `description`, `inputTokenLimit`, `outputTokenLimit`, **`supportedGenerationMethods[]`** (e.g. `generateContent`, `embedContent`), `thinking`, `temperature`, `maxTemperature`, `topP`, `topK`.
- Filter by `supportedGenerationMethods` (`embedContent` means an embedding model). Image models can be recognized by an ID containing `-image` (a heuristic; there is no explicit image flag).

### 2.3 IMAGE generation ("Nano Banana")

**2.3.1 Models** [G-MODELS][G-IMG][G-PRICING]

| Model ID | Name | Resolutions | Aspect ratios | Status |
|---|---|---|---|---|
| `gemini-3.1-flash-image` | Nano Banana 2 | 512 (0.5K), 1K (default), 2K, 4K | 1:1, 1:4, 4:1, 1:8, 8:1, 2:3, 3:2, 3:4, 4:3, 4:5, 5:4, 9:16, 16:9, 21:9 | Stable. "should be your go-to image generation model" |
| `gemini-3-pro-image` | Nano Banana Pro | 1K, 2K, 4K | 1:1, 2:3, 3:2, 3:4, 4:3, 4:5, 5:4, 9:16, 16:9, 21:9 | Stable. "professional asset production", thinking is always on |
| `gemini-3.1-flash-lite-image` | Nano Banana 2 Lite | **1K only** | 1:1, 3:2, 2:3, 3:4, 4:3, 4:5, 5:4, 9:16, 16:9, 21:9 | Stable. Fastest and cheapest. No Search grounding |
| `gemini-2.5-flash-image` | Nano Banana | ~1K | 10 ratios | **deprecated, shutdown 2026-10-02** |

- Image models have **no free tier** ("Free Tier: Not available"), so users need a billing-enabled project [G-PRICING].
- "All generated images include a SynthID watermark" (an invisible watermark).
- `imageSize` values must use an uppercase K (`1K`, `2K`, `4K`; also `512`). Lowercase is rejected.

**2.3.2 Exact output pixel sizes** (official table) [G-IMG]

| Ratio | NB2 1K | NB2 2K | **NB2 / Pro 4K** | Pro 1K | Pro 2K |
|---|---|---|---|---|---|
| 16:9 | 1376x768 | 2752x1536 | **5504x3072** | 1376x768 | 2752x1536 |
| 21:9 | 1584x672 | 3168x1344 | **6336x2688** | 1584x672 | 3168x1344 |
| 3:2 | 1264x848 | 2528x1696 | 5056x3392 | 1264x848 | 2528x1696 |
| 4:3 | 1200x896 | 2400x1792 | 4800x3584 | 1200x896 | 2400x1792 |
| 1:1 | 1024x1024 | 2048x2048 | 4096x4096 | 1024x1024 | 2048x2048 |

- Desktop mapping: 16:9 5K (5120x2880) is a downscale from 5504x3072. 16:10 can be cropped from 3:2 (5056x3392 to 5056x3160) or from 16:9. Ultrawide 5120x2160 is a downscale from 6336x2688.
- Output tokens per image are **the same for every aspect ratio** within a size tier. NB2: 747 / 1120 / 1680 / 2520 tokens. Pro: 1120 / 1120 / 2000.

**2.3.3 Pricing** (Paid tier, USD) [G-PRICING]

| Model | Per-image (Standard) | Batch | Other token prices |
|---|---|---|---|
| `gemini-3.1-flash-image` | 0.5K $0.045, 1K $0.067, 2K $0.101, **4K $0.151** ($60 / 1M image output tokens) | 0.5K $0.022, 1K $0.034, 2K $0.050, 4K $0.076 | in $0.50 / 1M; text+thinking out $3 / 1M |
| `gemini-3-pro-image` | 1K/2K $0.134, **4K $0.24** ($120 / 1M) | 1K/2K $0.067, 4K $0.12 (Flex same as Batch) | in $2.00; text+thinking out $12 |
| `gemini-3.1-flash-lite-image` | 1K $0.0336 ($30 / 1M) | $0.0168 | in $0.25; out text $1.50 |

Thinking tokens are billed. "Thought images" created during thinking are "not charged" [G-IMG].

**2.3.4 Requests**

*Interactions API* (recommended) [G-IMG]:
```bash
curl -s -X POST "https://generativelanguage.googleapis.com/v1beta/interactions" \
  -H "x-goog-api-key: $GEMINI_API_KEY" -H "Content-Type: application/json" \
  -d '{
    "model": "gemini-3.1-flash-image",
    "store": false,
    "input": "Ancient stone ruins at night in gentle rain, deep blue palette, ... no text.",
    "response_format": { "type": "image", "mime_type": "image/jpeg", "aspect_ratio": "16:9", "image_size": "4K" }
  }'
```
Response: an `Interaction` object `{id, model, object:"interaction", status:"completed", steps:[{type:"model_output", content:[...]}], usage:{...}}`. Image content blocks have `type: "image"` plus base64 `data` and `mime_type`. The SDKs expose this as `interaction.output_image.data` (base64) [G-API-INTERACTIONS].

*generateContent* (legacy, fully supported) [G-GC-IMG][G-GC-REF]:
```bash
curl -s -X POST "https://generativelanguage.googleapis.com/v1/models/gemini-3.1-flash-image:generateContent" \
  -H "x-goog-api-key: $GEMINI_API_KEY" -H "Content-Type: application/json" \
  -d '{
    "contents": [{ "role": "user", "parts": [{ "text": "Ancient stone ruins at night in gentle rain..." }] }],
    "generationConfig": {
      "responseModalities": ["IMAGE"],
      "imageConfig": { "aspectRatio": "16:9", "imageSize": "4K" }
    }
  }'
```
- The REST reference has both `generationConfig.imageConfig {aspectRatio, imageSize}` and a newer `generationConfig.responseFormat.image {aspectRatio, imageSize, mimeType, delivery}`. The current guide's REST examples use `responseFormat.image`. The default `responseModalities` is `["TEXT", "IMAGE"]`; set `["IMAGE"]` for image-only output.
- Response: `candidates[0].content.parts[]` with `inlineData {mimeType, data(base64)}`. Text parts may also appear.

**2.3.5 Other parameters, safety, and limits**
- **Negative prompt: not supported** on Nano Banana models (there is no field). Phrase exclusions positively in the prompt.
- **Seed:** `generationConfig.seed` exists ("Seed used in decoding"), but the docs make **no promise of reproducible images**. Treat it as best effort.
- **Thinking:** on by default for Gemini 3 image models and "cannot be disabled". For NB2 and NB2 Lite, `thinkingLevel` is `minimal` (default, lowest latency) or `high`. Pro can render up to two interim "thought images".
- **n images:** "The model won't always follow the exact number of image outputs". Make one request per image.
- **Blocked results (generateContent)** come back as a normal response, not an HTTP error:
  - `promptFeedback.blockReason` is one of `SAFETY | OTHER | BLOCKLIST | PROHIBITED_CONTENT | IMAGE_SAFETY`, and no candidates are returned.
  - Otherwise `candidates[].finishReason` is one of `IMAGE_SAFETY | IMAGE_PROHIBITED_CONTENT | IMAGE_OTHER | NO_IMAGE | IMAGE_RECITATION | PROHIBITED_CONTENT | SAFETY | SPII`.
  - Core harms such as child safety "are always blocked and cannot be adjusted" [G-SAFETY].
- Best prompt languages: EN, ar-EG, de-DE, es-MX, fr-FR, hi-IN, id-ID, it-IT, ja-JP, ko-KR, pt-BR, ru-RU, ua-UA, vi-VN, zh-CN [G-GC-IMG].
- Total inline request size limit: 20MB [G-IMG-UND].
- Latency: **not published.** Thinking adds latency. Use `thinkingLevel: minimal`.
- **Upscale: none** in the Gemini API (Imagen's upscale was never on the AI Studio path, and Imagen is shut down).

### 2.4 TEXT: structured output [G-SO][G-GC-SO][G-PRICING][G-M-35FL]

| Model | Input / Output per 1M | Notes |
|---|---|---|
| `gemini-3.5-flash-lite` (**recommended**) | $0.30 / $2.50 (Batch $0.15 / $1.25) | Stable. 1,048,576 in, 65,536 out. Structured outputs: Supported. Google says new projects should use "3.5 Flash-Lite or 3.8 Flash" |
| `gemini-3.1-flash-lite` (budget) | $0.25 / $1.50 | Stable. Listed as supporting structured output |
| `gemini-3.8-flash` | $0.75 / $3.75 until 2026-12-31, then $1.50 / $7.50 | newest Flash |

Output prices include thinking tokens. To minimize latency and cost, set `thinkingLevel: MINIMAL` (Gemini 3+) [G-GC-REF][G-TROUBLE].

*Interactions API:*
```json
POST /v1beta/interactions
{ "model": "gemini-3.5-flash-lite", "store": false,
  "input": "Keywords: rain, ruins, peaceful, night, blue ...",
  "response_format": { "type": "text", "mime_type": "application/json",
    "schema": { "type": "object",
      "properties": { "title": {"type":"string"}, "image_prompt": {"type":"string"},
                      "palette": {"type":"array","items":{"type":"string"}} },
      "required": ["title","image_prompt","palette"] } } }
```
The result is JSON text in `steps[].content[].text` (SDK: `interaction.output_text`).

*generateContent:*
```json
POST /v1beta/models/gemini-3.5-flash-lite:generateContent
{ "contents": [{ "parts": [{ "text": "..." }] }],
  "generationConfig": {
    "responseFormat": { "text": { "mimeType": "application/json", "schema": { ...JSON Schema... } } },
    "thinkingConfig": { "thinkingLevel": "MINIMAL" } } }
```
`responseSchema` and `_responseJsonSchema` / `responseMimeType` still exist but are marked **deprecated** in favor of `responseFormat`.

**Schema subset:**
- Supported: `string`, `number`, `integer`, `boolean`, `object`, `array`, `null` (via type arrays), `title`, `description`, `properties`, `required`, `additionalProperties`, `enum`, `format` (date-time, date, time), `minimum`, `maximum`, `items`, `prefixItems`, `minItems`, `maxItems`. `anyOf` and `$ref` appear in the examples.
- "The model ignores unsupported properties". "The API may reject very large or deeply nested schemas". Output order follows schema key order.
- Output is "syntactically valid JSON" with no semantic guarantee, so **validate it client-side**.

### 2.5 Embeddings [G-EMB][G-PRICING]
| Model | Input limit | Dims | Price |
|---|---|---|---|
| `gemini-embedding-2` (multimodal, stable, April 2026) | 8,192 tokens | 128–3072 (recommended 768/1536/3072). Default 3072. Truncations are auto-normalized | text $0.20 / 1M (Batch $0.10). Free tier available |
| `gemini-embedding-001` (text) | 2,048 tokens | 128–3072 | not on the current pricing page (see section 6) |

`POST /v1beta/models/gemini-embedding-2:embedContent` with `{"model": "models/gemini-embedding-2", "content": {"parts": [{"text": "..."}]}, "output_dimensionality": 768}`. `task_type` is not supported on embedding-2; put the task in the prompt instead. The two models' embedding spaces are incompatible.

### 2.6 Rate limits and errors [G-RL][G-TROUBLE]
- Limits are per **project** (not per key), measured in RPM, TPM, RPD, and IPM (images per minute) for image models. RPD resets at midnight Pacific. Exact numbers are shown only in AI Studio.
- Spend-based limits per 10 minutes: Tier 1 $10, Tier 2 $50, Tier 3 $200. Exceeding them returns `429 RESOURCE_EXHAUSTED`.
- Retry 429, 408, and 5xx (e.g. `503 UNAVAILABLE`) with exponential backoff and jitter. Do not retry 400/402/403.
- For Gemini 3.x, Google "strongly recommend[s]" keeping temperature, top_p, and top_k at their defaults.

### 2.7 Data retention / privacy (for PRIVACY.md) [G-TERMS][G-USAGE][G-INTERACTIONS]
- **Unpaid services** (free quota or AI Studio): Google "uses the content you submit... and any generated responses to provide, improve, and develop Google products". "Human reviewers may read, annotate, and process your API input and output." "Do not submit sensitive, confidential, or personal information." EEA, Switzerland, and UK users get paid-service terms even on the free tier.
- **Paid services** (project with active billing): Google "doesn't use your prompts... or responses to improve our products". It "logs prompts and responses for a limited period of time, solely for detecting and preventing violations of the Prohibited Use Policy".
- **Abuse monitoring retains prompts, context, and outputs for 55 days.** Flagged content may be human-reviewed.
- **Interactions API storage:** `store=true` by default, retained **55 days (paid)** or **1 day (free)**. Paid projects can set 7/14/28/55-day windows. Send **`store: false`** (incompatible with `background=true` and `previous_interaction_id`).
- The pricing table's "Used to improve our products" row reads Free = Yes, Paid = No for every model.

---

## 3. Local and OpenAI-compatible servers

### 3.1 Ollama [OLLAMA-*]
- **Default:** `http://localhost:11434`, binding 127.0.0.1. Change with `OLLAMA_HOST`. No auth locally [OLLAMA-FAQ].
- **Model list:**
  - `GET /api/tags` returns `{"models":[{"name","model","modified_at","size","digest","details":{"format","family","families","parameter_size","quantization_level"}}]}`.
  - OpenAI-style `GET /v1/models` also works.
  - `POST /api/show` returns `capabilities` (e.g. `"completion"`, `"vision"`, `"thinking"`). Use it to filter the picker.
- **Native structured output** (`POST /api/chat`): pass a JSON Schema in `format`. Ollama's docs recommend also putting the schema in the prompt:
```json
{ "model": "gemma4", "stream": false,
  "messages": [{ "role": "user", "content": "Keywords: rain, ruins ... Respond as JSON." }],
  "format": { "type": "object",
    "properties": { "title": {"type":"string"}, "image_prompt": {"type":"string"} },
    "required": ["title","image_prompt"] },
  "options": { "temperature": 0 } }
```
  - Response: `{"model":"...","message":{"role":"assistant","content":"{...json...}","thinking":"..."},"done":true,"done_reason":"stop"}`.
  - `format: "json"` gives schema-less JSON mode. Control thinking with `think` (true/false or a level; see `/api/show`).
- **OpenAI-compatible:** `POST http://localhost:11434/v1/chat/completions` supports `response_format`. The source maps `{"type":"json_schema","json_schema":{"schema":...}}` onto `format` [OLLAMA-OPENAI-GO]. Also supported: `seed`, `temperature`, `max_tokens`, `stream`, and `reasoning_effort`. Not supported: `n`, `tool_choice`, `logit_bias`, image URLs (base64 only) [OLLAMA-OAI]. Also available: `/v1/responses` and `/v1/embeddings`.
- **Ollama Cloud "does not support structured outputs"**, so warn users who point AutoPaper at a cloud model.
- **Embeddings:** `POST /api/embed` (local models).
- **Image generation status:**
  - Announced as experimental (macOS only) on 2026-01-20 with `x/z-image-turbo` and `x/flux2-klein` [OLLAMA-BLOG].
  - v0.32.5 (2026-07-27) exposed `POST /v1/images/generations` [OLLAMA-GH].
  - PR #16615 "mlx: remove experimental image generation code for now" was merged 2026-07-28, with the note "To be re-introduced on the new MLX runner".
  - Current v0.35.1 and v0.40.0-rc3 return **HTTP 400 `{"error":"image generation models are not currently supported"}`**. **Do not offer Ollama as an image backend.** Feature-detect it instead (send a request and treat 400/404 as unsupported).

### 3.2 LM Studio [LMS-*]
- **Server:** start with `lms server start` or the Developer tab. Examples "assume the server port is 1234" (the port is configurable). "Serve on Local Network" and "Enable CORS" are toggles.
- **Auth:** none by default. LM Studio ≥0.4.0 can require API tokens (`Authorization: Bearer $LM_API_TOKEN`).
- **OpenAI-compatible endpoints:** `GET /v1/models`, `POST /v1/responses`, `POST /v1/chat/completions`, `POST /v1/embeddings`, `POST /v1/completions`. Native REST: `/api/v1/chat`, `/api/v1/models`, plus load/unload/download. **No image generation endpoint.**
- **Structured output:** `/v1/chat/completions` with `"response_format": {"type": "json_schema", "json_schema": {"name": "...", "strict": "true", "schema": {...}}}`. The JSON string arrives in `choices[0].message.content`.
  - Engine: llama.cpp grammar-based sampling for GGUF, Outlines for MLX.
  - "Not all models are capable of structured output, particularly LLMs below 7B parameters."

### 3.3 Generic OpenAI-compatible `/v1/images/generations` servers
| Server | Default URL | Supported request fields | Extras |
|---|---|---|---|
| **LocalAI** [LOCALAI-IMG] | `http://localhost:8080/v1/images/generations` | `prompt`, `size`, `model`; extra `mode`, `step`, **`negative_prompt`** (or `"prompt": "positive\|negative"`) | Backends: `stablediffusion-ggml` (stable-diffusion.cpp, e.g. Flux GGUF) and `diffusers`. **Upscale:** `POST /v1/images/upscale` (multipart `model`, `scale`=2/4, `image`), returns an image under `/generated-images` |
| **stable-diffusion.cpp `sd-server`** [SDCPP-API] | `http://127.0.0.1:1234` (`--listen-ip`, `--listen-port`) | `prompt`, `n`, `size` (`WxH`), `output_format` (png/jpeg/webp), `output_compression`. Response `data[].b64_json`. **Extra params** (steps, seed, negative prompt, etc.) go in a `<sd_cpp_extra_args>{json}</sd_cpp_extra_args>` tag inside `prompt` | Also offers A1111-style `POST /sdapi/v1/txt2img` (with `negative_prompt`, `seed` (-1 = random), `batch_size`, hires `hr_scale` / `hr_resize_x`/`y`) and a native async job API `/sdcpp/v1/*` |
| Ollama | | removed (see 3.1) | |
| LM Studio | | not supported | |

Port clash: sd-server and LM Studio both default to **1234**.

### 3.4 ComfyUI HTTP API [COMFY-ROUTES][COMFY-MSGS][COMFY-EX][COMFY-SRC]
- **Default:** `http://127.0.0.1:8188` (`--port 8188`, `--listen 127.0.0.1`). `--enable-cors-header` is off by default. `--max-upload-size` defaults to 100 MB. There is no auth.
- **Flow:**
  1. Optionally `GET /object_info` (node schemas) and `GET /models/{folder}` (e.g. `checkpoints`) for a model picker. `GET /system_stats` shows devices and VRAM.
  2. Open a WebSocket to `ws://127.0.0.1:8188/ws?clientId=<uuid>`.
  3. Send `POST /prompt`:
     ```json
     { "prompt": { /* workflow in API format: {"<node_id>": {"class_type": "...", "inputs": {...}}} */ },
       "client_id": "<same uuid>",
       "prompt_id": "<optional canonical lowercase UUID; server mints one if omitted>" }
     ```
     Success: `{"prompt_id": "...", "number": <queue pos>, "node_errors": {}}`. Validation failure: **400** `{"error": {"type", "message", "details", "extra_info"}, "node_errors": {...}}`.
  4. Progress arrives over the WebSocket as JSON `{"type": ..., "data": {...}}`:
     - `status` (`exec_info.queue_remaining`), `execution_start`, `execution_cached`, `executing` (`data.node`; **`node == null` with your `prompt_id` means done**), `progress` (`value` / `max`), `executed`, `execution_success`, `execution_error`, `execution_interrupted`.
     - Binary frames are latent previews (an 8-byte header, then the image).
  5. Without a WebSocket: poll `GET /history/{prompt_id}` until the key appears. It returns `{prompt_id: {"outputs": {node_id: {"images": [{"filename", "subfolder", "type"}]}}, ...}}`.
  6. Download with `GET /view?filename=...&subfolder=...&type=output`.
  7. Cancel with `POST /interrupt`. Manage the queue with `POST /queue` and history with `POST /history`.
- **Templating prompt, size, and seed:** keep an API-format workflow JSON (from "Export (API)" in the UI). Before posting, overwrite specific node inputs, as the official example does:
  - positive `CLIPTextEncode.inputs.text`
  - negative `CLIPTextEncode.inputs.text`, giving **real negative prompt support**
  - `EmptyLatentImage.inputs.width/height/batch_size`
  - `KSampler.inputs.seed/steps/cfg/sampler_name/scheduler`
  - `CheckpointLoaderSimple.inputs.ckpt_name`
  - `SaveImage.inputs.filename_prefix`

  The app needs a per-template map from roles (`positive`, `negative`, `width`, `height`, `seed`, `steps`, `model`) to `(node_id, input_name)`, because node IDs are arbitrary per workflow.
- **Upscale in-graph:** `UpscaleModelLoader` and `ImageUpscaleWithModel` (core nodes in `comfy_extras/nodes_upscale_model.py`) take a 3840x2160 output to 5K/6K/8K.

---

## 4. Cross-cutting guidance

### 4.1 Timeouts and request sizes
| Call | Documented fact | Suggested client setting |
|---|---|---|
| OpenAI image | "up to 2 minutes". Prompt ≤32,000 chars. b64 PNG at 3840x2160 can be tens of MB | 180 s timeout, or stream with `partial_images`. Prefer `output_format: "jpeg"`/`"webp"` for speed and size |
| OpenAI text (Luna) | first-schema latency penalty | 60 s |
| Gemini image | latency not published. Inline request ≤20 MB | 180 s. `thinkingLevel: minimal` |
| Gemini text | | 60 s |
| ComfyUI / local | GPU dependent | No hard timeout. Use WebSocket progress, `/interrupt` to cancel |

### 4.2 Safety and abuse handling (app-side)
- Both cloud providers filter inputs and outputs themselves, and neither lets a client disable child-safety filtering. OpenAI lets you lower to `moderation: "low"`. On Gemini, custom `safetySettings` exist only on generateContent; core harms are always blocked.
- Map refusals to one generic UI message and keep the details in logs:
  - OpenAI image: `error.code == "moderation_blocked"`, `moderation_details.categories`
  - OpenAI text: a `refusal` content item
  - Gemini: `promptFeedback.blockReason` or `finishReason` in `IMAGE_*` / `SAFETY` / `PROHIBITED_CONTENT`
- Don't auto-retry a refused prompt unchanged. Optionally ask the text model to "soften" the concept and retry once.
- OpenAI usage policies and Google's Generative AI Prohibited Use Policy apply to the user, who holds the key. Say so in the ToS/PRIVACY.

### 4.3 PRIVACY.md cheat-sheet
| Provider | Training on API data | Retention | App controls |
|---|---|---|---|
| OpenAI | No (unless the org opts in) | abuse logs ≤30 days. Responses `store` defaults to true (≥30 days) | send `store:false`. Optional hashed `user` |
| Gemini paid | No | abuse logs 55 days. Interactions stored 55 days by default | send `store:false`. Recommend billing-enabled keys |
| Gemini free/unpaid | **Yes, and human review** | as above (Interactions 1 day) | warn users. EEA/CH/UK get paid terms |
| Ollama / LM Studio / ComfyUI / LocalAI / sd.cpp | local | local only | bind to 127.0.0.1. Warn if the user points to a LAN host |

---

## 5. Exact desktop-target cheat sheet (what to request)
| Display | OpenAI (2.5 Flare) request | Gemini (NB2/Pro) request | Post-process |
|---|---|---|---|
| 1920x1080 | `1920x1088` (crop 8px) | 16:9 2K (2752x1536) | downscale |
| 2560x1440 | `2560x1440` | 16:9 2K or 4K | downscale |
| 2560x1600 / 2880x1800 (16:10) | `2560x1600` / `3632x2272` | 3:2 4K (5056x3392) | crop to 16:10, downscale |
| 3440x1440 (21:9) | `3440x1440` | 21:9 2K (3168x1344) or 4K (6336x2688) | 4K downscale or 2K upscale |
| 3840x2160 (4K) | `3840x2160` (experimental) | 16:9 4K (5504x3072) | downscale |
| 5120x2880 (5K) | `3840x2160` + local upscale | 16:9 4K (5504x3072) | downscale |
| 6016x3384 (6K) | `3840x2160` + upscale | 16:9 4K + upscale (~1.1x) | upscale |
| 5120x2160 (ultrawide 5K2K) | `3840x1648` + upscale | 21:9 4K (6336x2688) | downscale |

---

## 6. Uncertain / unverified
1. **HTTP status code for OpenAI `moderation_blocked`:** the docs show only the error body. It is probably a 400, but that is unconfirmed.
2. **Gemini Interactions API image block / error shape:** documented only for generateContent (`promptFeedback`, `finishReason`). The Interactions equivalent (e.g. `status: "failed"` or an error object) was not found.
3. **Gemini Interactions image content field names:** `data` and `mime_type` in `steps[].content[]` are inferred from the API-reference type listing and the SDK property `output_image.data`. A raw JSON example of an image step was not found.
4. **Gemini `imageConfig` vs `responseFormat.image`:** both appear in the official REST reference. It is unclear whether `imageConfig` is being deprecated. Test both against `/v1` and `/v1beta`.
5. **Gemini seed determinism for images:** `generationConfig.seed` exists, but image reproducibility is not documented. The Interactions API seed field was not checked.
6. **Gemini image latency and per-model rate limits (IPM):** not published. They are shown only in AI Studio per project.
7. **Gemini 0.5K 21:9 size:** the official table lists "792x168", which is inconsistent with 21:9 and probably a doc typo. Don't use 0.5K for 21:9.
8. **`gemini-embedding-001` price:** missing from the current pricing page (it was formerly $0.15 / 1M); unverified.
9. **`gemini-3.1-flash-image-preview` / `gemini-3-pro-image-preview` aliases:** the rate-limits page still uses "Preview" names. Whether the old `-preview` IDs still resolve was not checked. Use the stable IDs.
10. **Structured-output support table on Google's page** doesn't list 3.5 Flash-Lite or 3.8 Flash, but their model pages say "Structured outputs: Supported" and the official example uses `gemini-3.8-flash`. Treat them as supported.
11. **OpenAI Flare "50% lower latency than GPT-Image-2":** from OpenAI's X/LinkedIn launch posts as quoted by third parties, not from the docs page.
12. **OpenAI 16:10 size `3632x2272`:** derived from the documented rules (multiple of 16, ≤3840 edge, ≤8,294,400 px). It is not an example in the docs. Cost figures for any size other than the three standard ones are computed with OpenAI's calculator formula, extracted from its JS. Verify against the `usage.output_tokens` of a real call.
13. **LocalAI response format** (`url` vs `b64_json` default, `response_format` support): not confirmed on the current page. The upscale endpoint "returns the generated image under /generated-images", which suggests URL output by default.
14. **ComfyUI Desktop default port 8000:** stated only by comfyui-wiki.com (unofficial). The ComfyUI server default `--port` is 8188 (verified in `comfy/cli_args.py`). Let users enter the port.
15. **Ollama image generation return:** the PR says "to be re-introduced on the new MLX runner"; no date given.
16. **Gemini abuse-monitoring retention for image outputs:** the usage-policy page lists "Prompts", "Contextual Information", and "Output" for 55 days, without mentioning images separately.

---

## 7. Sources (all fetched 2026-10-05)
- [OAI-IMG] https://developers.openai.com/api/docs/guides/image-generation (calculator JS: https://developers.openai.com/_astro/GptImageTokenCalculator.react.CH4dSfjq.js)
- [OAI-IMG-REF] https://developers.openai.com/api/reference/resources/images/methods/generate
- [OAI-FLARE] https://developers.openai.com/api/docs/models/gpt-image-2.5-flare
- [OAI-SUNBURST] https://developers.openai.com/api/docs/models/gpt-image-2.5-sunburst
- [OAI-GPTIMG2] https://developers.openai.com/api/docs/models/gpt-image-2
- [OAI-PRICING] https://developers.openai.com/api/docs/pricing
- [OAI-MODELS] https://developers.openai.com/api/docs/models
- [OAI-LUNA] https://developers.openai.com/api/docs/models/gpt-6-luna
- [OAI-GPT6] https://developers.openai.com/api/docs/guides/latest-model
- [OAI-SO] https://developers.openai.com/api/docs/guides/structured-outputs
- [OAI-RESP-REF] https://developers.openai.com/api/reference/resources/responses/methods/create
- [OAI-MODELS-LIST] https://developers.openai.com/api/reference/resources/models/methods/list
- [OAI-EMB] https://developers.openai.com/api/docs/guides/embeddings
- [OAI-DATA] https://developers.openai.com/api/docs/guides/your-data
- [OAI-ERRORS] https://developers.openai.com/api/docs/guides/error-codes
- [OAI-CHANGELOG] https://developers.openai.com/api/docs/changelog
- [G-MODELS] https://ai.google.dev/gemini-api/docs/models
- [G-IMG] https://ai.google.dev/gemini-api/docs/image-generation (Interactions variant)
- [G-GC-IMG] https://ai.google.dev/gemini-api/docs/generate-content/image-generation
- [G-IMAGEN] https://ai.google.dev/gemini-api/docs/imagen
- [G-PRICING] https://ai.google.dev/gemini-api/docs/pricing
- [G-SO] https://ai.google.dev/gemini-api/docs/structured-output
- [G-GC-SO] https://ai.google.dev/gemini-api/docs/generate-content/structured-output
- [G-GC-REF] https://ai.google.dev/api/generate-content
- [G-API-INTERACTIONS] https://ai.google.dev/api/interactions-api
- [G-INTERACTIONS] https://ai.google.dev/gemini-api/docs/interactions
- [G-API-MODELS] https://ai.google.dev/api/models
- [G-EMB] https://ai.google.dev/gemini-api/docs/embeddings
- [G-RL] https://ai.google.dev/gemini-api/docs/rate-limits
- [G-TROUBLE] https://ai.google.dev/gemini-api/docs/troubleshooting
- [G-SAFETY] https://ai.google.dev/gemini-api/docs/safety-settings
- [G-TERMS] https://ai.google.dev/gemini-api/terms
- [G-USAGE] https://ai.google.dev/gemini-api/docs/usage-policies
- [G-IMG-UND] https://ai.google.dev/gemini-api/docs/image-understanding
- [G-M-35FL] https://ai.google.dev/gemini-api/docs/models/gemini-3.5-flash-lite (also /gemini-3.1-flash-image, /gemini-3.1-flash-lite-image)
- [OLLAMA-SO] https://docs.ollama.com/capabilities/structured-outputs
- [OLLAMA-OAI] https://docs.ollama.com/openai (and /api/openai-compatibility)
- [OLLAMA-API] https://docs.ollama.com/api/chat, https://docs.ollama.com/api/tags, https://docs.ollama.com/api/generate
- [OLLAMA-FAQ] https://docs.ollama.com/faq
- [OLLAMA-BLOG] https://ollama.com/blog/image-generation
- [OLLAMA-GH] https://github.com/ollama/ollama/blob/v0.35.1/server/routes.go (line 444), https://github.com/ollama/ollama/pull/16615, https://github.com/ollama/ollama/blob/v0.32.5/server/routes.go (line 1916, `/v1/images/generations`)
- [OLLAMA-OPENAI-GO] https://github.com/ollama/ollama/blob/main/openai/openai.go (json_schema mapping, ~line 755)
- [LMS-OC] https://lmstudio.ai/docs/developer/openai-compat
- [LMS-SO] https://lmstudio.ai/docs/developer/openai-compat/structured-output
- [LMS-REST] https://lmstudio.ai/docs/developer/rest
- [LMS-AUTH] https://lmstudio.ai/docs/developer/core/authentication
- [LMS-SETTINGS] https://lmstudio.ai/docs/developer/core/server/settings
- [LOCALAI-IMG] https://localai.io/docs/features/image-generation/
- [SDCPP-API] https://github.com/leejet/stable-diffusion.cpp/blob/master/examples/server/api.md (and README.md)
- [COMFY-ROUTES] https://docs.comfy.org/development/comfyui-server/comms_routes
- [COMFY-MSGS] https://docs.comfy.org/development/comfyui-server/comms_messages
- [COMFY-EX] https://github.com/Comfy-Org/ComfyUI/blob/master/script_examples/websockets_api_example.py
- [COMFY-SRC] https://github.com/Comfy-Org/ComfyUI/blob/master/server.py (POST /prompt handler), https://github.com/Comfy-Org/ComfyUI/blob/master/comfy/cli_args.py (port 8188), comfy_extras/nodes_upscale_model.py
