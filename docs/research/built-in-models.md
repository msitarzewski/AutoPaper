# AutoPaper: the operating systems' built-in models (text and image)

Research date: 2026-10-06. This answers one question: can the models that ship with macOS, Windows and Linux do AutoPaper's two
jobs?

- **Writing:** turn keywords into 4 structured JSON concept candidates. The real request (captured from `autopaper compose`)
  is a 6,983-character system prompt plus an 888-character user prompt plus a 2,083-character JSON Schema, and the answer
  is about 1,400 tokens.
- **Painting:** a landscape image for a desktop, ideally 2K or larger.

Facts come from official documentation (developer.apple.com DocC JSON, learn.microsoft.com, WWDC26 session 241,
machinelearning.apple.com, support.apple.com), from the macOS 27.2 SDK's `.swiftinterface` files, or from probes run on this
Mac. Probe results are marked **[measured]**. Anything that could not be confirmed is in section 5. Citation tags point to
section 6.

## 0. Verdict

| | Writing (4 JSON candidates) | Painting (2K+ landscape) |
|---|---|---|
| **macOS 27 (Apple Intelligence)** | **PARTLY.** It works, but quality is below the hosted models. Foundation Models' on-device model returns schema-valid JSON for the real composer request, and the composer's own JSON Schema decodes straight into `GenerationSchema` **[measured]**. The context window is 8,192 tokens on AFM 3 Core Advanced (this M5 Max) and 4,096 on AFM 3 Core. A request takes ≈ 2,450 tokens in and ≈ 1,400 out, 32–49 s. Instruction-following is weak: without a regex guide, 0 of 16 candidates from the real instructions put the Must keyword in the image prompt **[measured]**. Works sandboxed, without entitlements, from a background agent app **[measured]**. Private Cloud Compute (32K, reasoning) requires App Store distribution and a managed entitlement | **CAN'T.** `ImageCreator` is deprecated and throws `notSupported` on macOS 27 [A-IC-INIT], **[measured]**. Image Playground's models moved to Private Cloud Compute [A-MLR]. All that remains is the interactive sheet or view controller, which needs a person to click and tops out at 2560×1440 **[measured]** |
| **Windows 11 (Windows AI APIs)** | **PARTLY, and not worth adopting now.** Phi Silica has `GenerateStructuredJsonResponseAsync(prompt, jsonSchema, options)` in stable WinAppSDK 2.3.1+ [MS-RN2]. But it needs a Limited Access Feature token, a Copilot+ NPU (or an RTX 30+/RX 9060+ GPU on an Insider Experimental build with Developer Mode), and a packaged app with `systemAIModels`. The context size is undocumented. Phi Silica is removed in January 2027 in favour of Aion Instruct [MS-PHI] | **PARTLY, and only on paper.** `ImageGenerator` (Stable Diffusion/SDXL) does text-to-image, but only on the Copilot+ NPU, after an optional download of several GB. It ships only in WinAppSDK **2.0 Experimental** (`[Experimental]` attribute), and has no size option [MS-IMG][MS-IMGOPT] |
| **Linux** | **CAN'T** at OS level (there is none). Ollama, which AutoPaper already supports, is the answer | **CAN'T** at OS level. ComfyUI or stable-diffusion.cpp, both already supported |
| **Parallels ARM64 VM (no NPU)** | Phi Silica: no (needs an NPU or a supported discrete GPU). Foundry Local's CPU execution provider should run (unverified in the VM) | No: `ImageGenerator` and `ImageScaler` are NPU-only |

Upscaling: Windows `ImageScaler` (stable, NPU-only, up to 8×) [MS-ISR]. On macOS, VideoToolbox `VTSuperResolutionScaler`
does **4× only**, takes image inputs up to 1920×1920, and needs a one-time model download **[measured]** [A-VTSR].

---

## 1. Apple (macOS 27.2, M5 Max; Xcode-beta SDK 27.2, Swift 6.4)

### 1.1 Foundation Models: availability

| Item | Fact | Source |
|---|---|---|
| OS | `SystemLanguageModel` macOS 26.0+. `PrivateCloudComputeLanguageModel`, `Variant` and `ContextOptions.reasoningLevel` macOS 27.0+. `contextSize` and `tokenCount(for:)` 26.4+ (`contextSize` is back-deployed to 26.0, where it returns 4096) | SDK `.swiftinterface`; [A-CTX] |
| Macs | Apple Intelligence needs "Mac with M1 or later". Storage is "up to 14 GB" on "M3 and later with at least 12GB of unified memory" and up to 8 GB on others | [A-SUP] |
| Model versions | Three on-device model versions so far: 26.0–26.3, 26.4, and 27.0. "Because the model changes when a person updates… test your prompts" | [A-SLM][A-UPD] |
| Variants (27) | `SystemLanguageModel.Variant.core3` = "AFM 3 Core" (3B dense). `.coreAdvanced3` = "AFM 3 Core Advanced" (20B sparse, 1–4B active), which "won't be available to all devices" | [A-VAR][A-MLR] |
| Unavailable reasons | `.deviceNotEligible`, `.appleIntelligenceNotEnabled`, `.modelNotReady` (model downloading). Also `SystemLanguageModel.Error.assetsUnavailable` if Apple Intelligence is switched off mid-run | [A-UR][A-AU] |
| Languages | `supportedLanguages` returned 24 locales here: da, de, en (+AU/GB/IN), es (+419/US), fr (+CA), it, ja, ko, nb, nl, pt (+PT), sv, tr, vi, zh (+HK/TW). Apple lists 16 languages. Not in China mainland for China-bought devices with China Apple Accounts | **[measured]**; [A-SUP] |
| Entitlement (on-device) | **None.** It worked from an unsigned CLI and from an ad-hoc-signed app with only `com.apple.security.app-sandbox` | **[measured]** |

**[measured] on this Mac:** `availability = .available`, `variant = "AFM 3 Core Advanced"`, **`contextSize = 8192`**,
capabilities `guidedGeneration`, `vision` and `toolCalling` (not `reasoning`).

### 1.2 The context window: 4,096 or 8,192

- The docs still say 4K. "Managing the context window" says "a context window of 4096 tokens per session", and the PCC
  article's table says "Context size: 4K" [A-MCW][A-PCC].
- The SDK makes it per-variant on 27. `contextSize` returns `_contextSize` on macOS 27 and a constant `4096` before that
  (SDK `.swiftinterface`).
- WWDC26 session 241 shows `print(model.contextSize) // 8192` [A-WWDC241]. This Core Advanced Mac returns 8192
  **[measured]**.
- **Read `contextSize` at runtime.** Assume 4,096 for AFM 3 Core devices (which ones those are is unverified; see 5).

AutoPaper's request against that window, counted with Apple's tokenizer **[measured]**:

| Part | Tokens |
|---|---|
| System instructions (`composer` system prompt) | 1,642 |
| User prompt | 213 |
| JSON Schema for 4 candidates (sent in the prompt by default) | 594 |
| Answer, 4 candidates | 1,043–1,433 |
| **Total** | **≈ 3,500–3,900** |

That total fits 8,192 easily, but it is right at the edge of 4,096. A retry adds the "too close to past wallpapers" text,
which pushes it over.

### 1.3 Guided generation with a runtime schema

- `DynamicGenerationSchema` builds object, array (with `minimumElements` and `maximumElements`), anyOf and reference schemas
  at runtime. `GenerationSchema(root:dependencies:)` turns them into a schema, and
  `session.respond(to:schema:options:)` returns `GeneratedContent` (`.jsonString`) [A-DGS].
- **`GenerationSchema` is `Codable`.** `JSONDecoder().decode(GenerationSchema.self, from: <composer::schema JSON>)`
  **succeeded** on AutoPaper's exact schema and generated correctly **[measured]**. So the host can pass the core's schema
  through unchanged.
- Guides:
  - `.pattern(Regex)` works only in simple forms. `"Misty mountains .+"` forced every prompt to start with the phrase, 4/4
    **[measured]**. Its side effect: the output read "Misty mountains . A weathered…".
  - These fail with `UnsupportedGuide`: `{m,n}` quantifiers, and a JSON Schema `"pattern": "^.*x.*$"` **[measured]**.
  - `".*misty mountains.*"` ran away: it hit the 2,400-token cap and returned 3 candidates of 206–600 words in 130 s
    **[measured]**.
  - `.constant` and `.anyOf` also exist for strings (SDK).
- Options are `GenerationOptions(samplingMode:temperature:maximumResponseTokens:)`:
  - `temperature` "must be a number between 0 and 1 inclusive" [A-TEMP]. AutoPaper's 0.4–1.2 must be clamped.
  - `SamplingMode.greedy`, `.random(top:seed:)` and `.random(probabilityThreshold:seed:)` support a seed (SDK).

### 1.4 Speed and quality: the real composer request on AFM 3 Core Advanced **[measured]**

| Run | Time | Tokens in / out | Prompt words | Must keyword "misty mountains" in prompt |
|---|---|---|---|---|
| 1 candidate, short instructions | 6.0 s | 660 / 239 | 44 | no ("Misty peaks") |
| Full request, decoded JSON Schema | 48.9 s | 2,448 / 1,387 | 93–113 | 0/4 (no candidate even says "mountain") |
| Full request, `DynamicGenerationSchema` | 39.7 s | 2,441 / 1,433 | 66–71 | 0/4 |
| Full + Must phrase in the `prompt` field's description (×2) | 31.7 / 40.2 s | 2,459 / 1,043–1,191 | 52–91 | 0/8; Maybe "lighthouse" 0/8 |
| Compact 156-token instructions, 4 candidates | 21.6 s | 785 / 915 | 33–38 (rule: 60–120) | 2/4 |
| Compact instructions, 2 candidates | failed | — | — | "Content contains 8193 tokens, which exceeds… 8192" (runaway) |
| Full + regex prefix guide | 44.1 s | 2,227 / 1,359 | 111–124 | 4/4 (forced) |

Every response was valid JSON with exactly the requested number of candidates, except the two runaways. There were no
guardrail violations or refusals on these innocent scenes. The default guardrails check input and output, and
`permissiveContentTransformations` only relaxes `String` output, not guided generation [A-GR][A-SAFE]. Throughput was about
30–36 output tokens/s.

### 1.5 Sandboxed menu-bar app and background work **[measured]**

- I built an `LSUIElement` app bundle, ad-hoc signed with only the App Sandbox entitlement. It set
  `NSApplication.setActivationPolicy(.accessory)` and was launched hidden with `open -g` (`isActive = false`).
- Result: availability `.available`, `contextSize` 8192, and 3/3 small schema-guided requests succeeded in 1.8–2.6 s.
- So a sandboxed menu bar agent can compose while it is not frontmost.
- The documented risk is `LanguageModelError.rateLimited` ("too many requests in a short window… spacing your requests")
  [A-RL]. AutoPaper makes one request per wallpaper, so this is not a concern.
- Probe side effect: the container `~/Library/Containers/dev.autopaper.research-probe` is protected by macOS and couldn't be
  deleted. It is harmless.

### 1.6 Private Cloud Compute (the larger server model)

| Item | Fact | Source |
|---|---|---|
| API | `LanguageModelSession(model: PrivateCloudComputeLanguageModel())` takes the same respond calls. 32K context, `ContextOptions(reasoningLevel: .light/.moderate/.deep)`, "Limit per day" per person, more with iCloud+ | [A-PCC][A-WWDC241] |
| Eligibility | Small Business Program **and** fewer than 2 million first-time downloads **and** the managed entitlement `com.apple.developer.private-cloud-compute`. "Apps distributed on the App Store" (TestFlight/ad hoc for testing). No cloud API cost | [A-PCCDEV][A-PCCENT] |
| On this Mac | `availability = .available`, `contextSize = 32768`, quota `belowLimit` were all readable. But `respond` from an un-entitled process failed with `LanguageModelError -1` / `ModelManagerError 1046` | **[measured]** |
| For AutoPaper | AutoPaper's Mac build updates through Sparkle (Developer ID), so it is **not eligible** unless a Mac App Store build is made | systemPatterns "Native surfaces" |

### 1.7 Painting: Image Playground

| Item | Fact | Source |
|---|---|---|
| `ImageCreator` (programmatic) | `@available(anyAppleOS, deprecated: 27.0, message: "Use ImagePlaygroundViewController or imagePlaygroundSheet.")`. The doc note says "`ImageCreator.Error.notSupported` on iOS 27.0+, macOS 27.0+". `try await ImageCreator()` threw `notSupported` here | SDK; [A-IC-INIT]; **[measured]** |
| Why | Image generation is now "ADM 3 Cloud" on Private Cloud Compute ("the all-new Image Playground") | [A-MLR] |
| Background | Even before 27, `backgroundCreationForbidden`: "Apps must perform image creation only when running in the foreground" | [A-BCF] |
| Styles (27.2) | `any` (new in 27: "style inferred from the prompt"), `animation`, `illustration` ("2D cartoon"), `sketch`, `emoji`, `externalProvider` (id `z_external_provider`) | SDK; [A-STY]; **[measured]** |
| Sizes (27) | `SizeSpecification.closest(to:)` over a "finite set of sizes": 1024×1024 → 1024×1024; 1920×1080 → 1312×736; 2560×1440, 3840×2160 and 5120×2880 → **2560×1440**; 3440×1440 and 2560×1600 → 2560×1440 (no 21:9 or 16:10). The default is "the smallest image size with a square aspect ratio" | [A-SIZE]; **[measured]** |
| What's left | `imagePlaygroundSheet` / `ImagePlaygroundViewController`. A person must take part, so it can't serve scheduled generation | [A-IP] |

### 1.8 Upscaling on macOS

| API | Fit for wallpapers | Source |
|---|---|---|
| VideoToolbox `VTSuperResolutionScalerConfiguration` (macOS 26+) | ML super-resolution with `inputType: .image`. Image input up to 1920 wide × 1920 high on macOS. `supportedScaleFactors = [4]` only. 1920×1080 → 7680×4320 and 1280×720 → 5120×2880 configs were accepted. 2560×1440 ×2 was rejected. Models need `downloadConfigurationModel` ("drive download with user awareness"); status here was `downloadRequired`. A 2560×1440 source needs tiling or a downscale first | Header; [A-VTSR]; **[measured]** |
| MetalFX `MTLFXSpatialScaler` | A real-time game upscaler (not ML, any texture). Usable for stills, but quality for wallpapers is unverified | SDK headers |
| Core Image / Vision | No super-resolution filter in the 27.2 SDK (grep) | SDK |

---

## 2. Microsoft (Windows 11, Windows App SDK 2.x; AutoPaper pins 2.5.1 stable)

### 2.1 What exists [MS-APIS] (page dated 2026-10-02)

| API | NPU (Copilot+) | GPU | CPU | Model delivery |
|---|---|---|---|---|
| Phi Silica (`Microsoft.Windows.AI.Text.LanguageModel`) | ✅ | ✅ NVIDIA RTX 30+ / AMD RX 9060+ (6 GB+), Insider Experimental build 26300.8553+, WinAppSDK 2.2.2-experimental9, Developer Mode | ❌ | Preinstalled on NPU. Several GB on demand for GPU |
| Image Generation (`Microsoft.Windows.AI.Imaging.ImageGenerator`) | ✅ (optional, removable) | ❌ | ❌ | Several GB on demand through Windows Update |
| Image Super Resolution (`ImageScaler`) | ✅ | ❌ | ❌ | — |
| Image Description, Segmentation, Object Erase, OCR | ✅ | ❌ | ❌ | — |

All of them require a packaged MSIX app with `<systemai:Capability Name="systemAIModels"/>` and `MaxVersionTested` ≥
10.0.26226.0 [MS-GS][MS-IMG]. AutoPaper's template already declares `systemAIModels` (docs/research/windows.md).
`AICapabilities.HasAICapability` (WinAppSDK 2.1.3) tells whether the device is a Copilot+ PC [MS-RN2].

### 2.2 Writing: Phi Silica

- **Limited Access Feature:** "The Phi Silica APIs are part of a Limited Access Feature… request an unlock token". This has
  been enforced since WinAppSDK 2.0.1 [MS-PHI][MS-RN2]. Not available in China.
- **Structured output:** `LanguageModel.GenerateStructuredJsonResponseAsync(string prompt, string jsonSchema[, LanguageModelOptions])`,
  "constrained to a caller-supplied JSON Schema". It arrived in stable **2.3.1** (2026-07-16). The docs don't list the
  supported JSON Schema subset. The method takes no separate system prompt or context, so system and user text would be
  concatenated [MS-SJ][MS-RN2].
- **Options:** `Temperature`, `TopP`, `TopK`, `ContentFilterOptions`, `LowRankAdapter` [MS-LMO].
- **Context:** no size is documented. Use `GetUsablePromptLength(prompt)`. Prompt compression for longer context is
  NPU-only [MS-LM][MS-PHI].
- **Moderation:** on by default at `medium` (severity 0–3 returned). It can be made stricter but not looser; "high: Not
  available" [MS-CM].
- **Being replaced:** "Phi Silica is being replaced by Aion Instruct… November 2026 Insider… January 2027 retail devices
  and Phi Silica is removed… LAF tokens are no longer needed with Aion Instruct" [MS-PHI]. The Edge blog says Aion reaches
  "devices without a GPU" through CPU inference, but that is in Edge [MS-EDGE]. Its hardware matrix under the Windows API
  is unverified.

### 2.3 Painting: Image Generation

- Text-to-image, image-to-image, Magic Fill, Coloring Book and Restyle, "Stable Diffusion-powered". The tip calls the model
  "SDXL" [MS-IMG].
- Prerequisites: Windows 11 24H2+, **Windows App SDK 2.0 Experimental**, and a Copilot+ NPU (required). The API reference
  marks the classes `[Windows.Foundation.Metadata.Experimental]` with the moniker `windows-app-sdk-2.0-experimental` only.
  None of the 2.x stable release notes mention it [MS-IMGOPT][MS-RN2]. **It can't ship in AutoPaper's stable 2.5.1 build.**
- `ImageGenerationOptions` has only `MaxInferenceSteps`, `Creativity`, `Seed` and `ContentFilterOptions`.
  `ImageFromTextGenerationStyle` is `Default` or `ColoringBook`. **There is no width, height or aspect option**
  [MS-IMGOPT][MS-IMGSTY], so output size is unverified (see 5).

### 2.4 Upscaling: Image Super Resolution (could feed a 4K/5K display)

`ImageScaler.ScaleSoftwareBitmap(bitmap, targetWidth, targetHeight)` / `ScaleImageBuffer`, with a `MaxSupportedScaleFactor`
property. "Scaling is limited to a maximum factor of 8x". It is stable (WinAppSDK 1.7 onwards, including 2.0) and runs on
the NPU only [MS-ISR][MS-ISRAPI][MS-APIS]. 2560×1440 → 5120×2880 is 2×, well inside the limit, but only on Copilot+ PCs.

### 2.5 Foundry Local and Windows ML

| Item | Fact | Source |
|---|---|---|
| What it is | An "end-to-end local AI solution", mainly an **in-process SDK** (C#, JavaScript, **Rust** `foundry-local-sdk`, Python) of about 20 MB on ONNX Runtime. Curated catalog (GPT OSS, Qwen, DeepSeek, Mistral, Phi, Whisper). Windows, macOS (Apple silicon) and Linux | [MS-FL] |
| OpenAI-compatible server | "Optional local server". The CLI (**preview**): `foundry server start [--port <p>] [--idle-timeout 0]`. The port is dynamic unless set; `foundry server status` shows the URL. `POST /v1/chat/completions` is "fully compatible with the OpenAI Chat Completions API". Models are listed at `GET /openai/models` / `GET /foundry/list` | [MS-FLCLI][MS-FLREST] |
| Install | `winget install Microsoft.FoundryLocal`. macOS: `brew tap microsoft/foundrylocal && brew install foundrylocal` | [MS-FLCLI] |
| Hardware | The CPU execution provider (MLAS) "run[s] on any CPU" and is the fallback. WebGPU covers any GPU. Plugin EPs for QNN, OpenVINO, TensorRT-RTX and VitisAI | [MS-FLCLI] |
| Structured output | `response_format` is **not** in the documented request fields (unverified; see 5) | [MS-FLREST] |
| Licences | Per model (`foundry model list --verbose` shows the License column) plus the EP vendor licences | [MS-FLCLI] |
| Windows ML | The Windows-maintained ONNX Runtime with managed execution providers. A runtime, **not a model** | [MS-WML] |

Can AutoPaper's existing OpenAI-compatible provider just point at it? **Mostly yes, for text.** Start
`foundry server start --port <fixed> --idle-timeout 0` and use base URL `http://127.0.0.1:<port>/v1`. Two caveats:
- Whether `response_format: json_schema` is honoured is undocumented.
- `GET /v1/models` isn't documented (the list lives at `/openai/models`), so the model picker may need the ID typed in.

There is no image endpoint.

### 2.6 Parallels ARM64 VM (no NPU)

| API | In the VM | Why |
|---|---|---|
| Phi Silica | ❌ expected `NotSupportedOnCurrentSystem` | NPU, or RTX 30+/RX 9060+ with an IHV driver |
| ImageGenerator, ImageScaler | ❌ | NPU only |
| Foundry Local | likely ✅ on CPU (slow) | CPU EP "on any CPU". Not tried in the VM |
| Aion Instruct (from Jan 2027) | unknown | CPU support is described only for Edge |

---

## 3. Linux

- There is no OS-level model or API. Freedesktop and GNOME are only discussing "local AI models as shared desktop
  infrastructure" [LX-GNOME].
- Local options AutoPaper already supports:
  - Text: Ollama (Homebrew bottles for Linux; port 11434), LM Studio, llama.cpp `llama-server` (OpenAI-compatible).
  - Images: ComfyUI, stable-diffusion.cpp `sd-server`, LocalAI.
- Foundry Local lists Linux as supported [MS-FL], but the CLI reference gives install steps only for Windows and macOS.

---

## 4. Recommendation for AutoPaper

### 4.1 A new provider kind: `System` ("On this Mac" / "On this PC")

Text only for now. The host implements it through a UniFFI foreign trait, the same way as `SecretStore`:

```rust
#[uniffi::export(with_foreign)]
#[async_trait]
pub trait SystemModel: Send + Sync {
    fn status(&self) -> SystemModelStatus;          // available | reason (DeviceNotEligible, NotEnabled, NotReady, …),
                                                    // context_size, display name ("AFM 3 Core Advanced")
    async fn compose(&self, system: String, user: String, schema_json: String,
                     temperature: f32, max_output_tokens: u32) -> Result<SystemComposeResult, SystemModelError>;
                                                    // JSON text + input/output tokens
}
```

- **macOS host (Swift):**
  - Decode `schema_json` straight into `GenerationSchema` (verified). Use a fresh `LanguageModelSession(instructions:)` per
    request.
  - Clamp temperature to ≤ 1. Always set `maximumResponseTokens` (about 2,400), because runaways happen.
  - Map `contextSizeExceeded` → retry with fewer candidates; `guardrailViolation` / `refusal` → refused;
    `rateLimited` / `assetsUnavailable` → unavailable with reason.
  - No entitlement is needed. It works sandboxed and while the menu bar app is in the background.
- **Windows host:** wait for Aion Instruct (no LAF token, wider hardware) and verify `GenerateStructuredJsonResponseAsync` on
  it before wiring the same trait. Phi Silica isn't worth a LAF request three months before it is removed. Meanwhile
  **Foundry Local** goes through the existing OpenAI-compatible provider: a preset with the fixed-port hint and a manual
  model ID. That needs no new code, only a `response_format` check.
- **No `generate(prompt, w, h)` on the trait yet.**
  - macOS has no programmatic image generation on 27.
  - Windows `ImageGenerator` is Experimental-channel, Copilot+-only and size-less.
  - Revisit when it reaches a stable WinAppSDK.
  - A separate optional `upscale(bytes, w, h)` host hook could use Windows `ImageScaler` (Copilot+) and macOS
    `VTSuperResolutionScaler` (4×, ≤ 1920 input, needs a consented model download). Low priority: Gemini 4K already covers
    5K.

### 4.2 When it is the default

- On macOS, when `status()` is available, make "On this Mac" the default **writer instead of Demo** for people who haven't
  set up a provider. It is free, private and offline.
- **Don't** prefer it over a configured hosted or Ollama writer: it measured clearly worse at following the composer's
  rules.
- It never paints, so the painter default stays as it is (the demo or a configured provider).

### 4.3 What it means for the composer (an "on-device profile")

- **Fit the window:**
  - `contextSize` ≥ 8,192 (Core Advanced): 4 candidates fit (≈ 3.9K used).
  - `contextSize` = 4,096 (Core): use 2 candidates and trim instructions.
  - Count with the host (`tokenCount`) or budget about 1 token per 4 characters.
- **Shorter instructions:** about 400–600 tokens instead of 1,642. Long rule lists were not followed anyway (0/16 Must
  compliance).
- **The core repairs instead of only rejecting:**
  - Splice missing Must keywords into `prompt` itself. A regex prefix guide (`"<musts>, .+"`) works but is awkward, and
    `.*` patterns run away.
  - Relax the 60–120-word check: the model gave 33–124 words.
  - Keep the Avoid check strict ("people" never appeared in the 8 full-request candidates that were inspected).
- **Expect 20–50 s per compose** on Core Advanced. Show "Composing…" progress. The 60 s text timeout is too tight for a
  retry; give it about 120 s.
- **Re-test prompts on each OS model version** (26.4 → 27.0 changed the model) [A-UPD].

---

## 5. Unverified / open

1. **Which Macs get AFM 3 Core Advanced (8,192) versus Core (4,096).** Apple says only "our most capable Apple silicon
   systems". The support page's "M3 and later with at least 12GB" 14 GB storage tier probably marks Core Advanced, but this
   is inferred. Core's `contextSize` on 27 was not measured (no Core device here).
2. Whether `JSONDecoder` → `GenerationSchema` accepts arbitrary JSON Schema on **macOS 26.x**. It was verified on 27.2 only.
   Which keywords map to guides (`minItems`/`maxItems` were honoured; `pattern` was not) is undocumented.
3. Whether PCC's managed entitlement can be used by a **Developer ID** (non-App Store) app. The eligibility page names App
   Store distribution only.
4. Image Playground's sheet output pixels for `.closest(to: 2560×1440)` (requested size measured; no image was generated
   because the sheet needs a person). Terms for using Image Playground images as wallpapers were not researched, since
   it can't be automated.
5. Phi Silica / Aion context size, Aion's hardware support under the Windows AI APIs, and the JSON Schema subset
   `GenerateStructuredJsonResponseAsync` accepts.
6. Windows `ImageGenerator` output resolution (no size option exists; SDXL's native 1024×1024 is likely but undocumented)
   and whether it will reach a stable WinAppSDK.
7. Foundry Local:
   - whether `/v1/chat/completions` honours `response_format: {type: json_schema}`
   - whether `/v1/models` exists
   - whether it runs in the Parallels ARM64 VM (CPU EP)
   - the Linux install path
8. `VTSuperResolutionScaler` model download size and quality on AI-generated stills, and MetalFX spatial scaling quality.
9. Background rate limits for Foundry Models over many runs. Only 3 back-to-back background requests were tested.

---

## 6. Sources

Apple (DocC JSON: `https://developer.apple.com/tutorials/data/documentation/<path>.json`)
- [A-SLM] https://developer.apple.com/documentation/foundationmodels/systemlanguagemodel
- [A-CTX] https://developer.apple.com/documentation/foundationmodels/systemlanguagemodel/contextsize
- [A-VAR] https://developer.apple.com/documentation/foundationmodels/systemlanguagemodel/variant-swift.struct
- [A-UR] https://developer.apple.com/documentation/foundationmodels/systemlanguagemodel/availability-swift.enum/unavailablereason
- [A-AU] https://developer.apple.com/documentation/foundationmodels/systemlanguagemodel/error/assetsunavailable(_:)
- [A-MCW] https://developer.apple.com/documentation/foundationmodels/managing-the-context-window
- [A-UPD] https://developer.apple.com/documentation/updates/foundationmodels
- [A-DGS] https://developer.apple.com/documentation/foundationmodels/dynamicgenerationschema
- [A-TEMP] https://developer.apple.com/documentation/foundationmodels/generationoptions/temperature
- [A-GR] https://developer.apple.com/documentation/foundationmodels/systemlanguagemodel/guardrails/permissivecontenttransformations
- [A-SAFE] https://developer.apple.com/documentation/foundationmodels/improving-the-safety-of-generative-model-output
- [A-RL] https://developer.apple.com/documentation/foundationmodels/languagemodelerror/ratelimited(_:)
- [A-PCC] https://developer.apple.com/documentation/foundationmodels/adding-server-side-intelligence-with-private-cloud-compute
- [A-PCCENT] https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.developer.private-cloud-compute
- [A-PCCDEV] https://developer.apple.com/private-cloud-compute/
- [A-WWDC241] https://developer.apple.com/videos/play/wwdc2026/241/ ("What's new in the Foundation Models framework")
- [A-MLR] https://machinelearning.apple.com/research/introducing-third-generation-of-apple-foundation-models
- [A-SUP] https://support.apple.com/en-us/121115 ("How to get Apple Intelligence")
- [A-IP] https://developer.apple.com/documentation/imageplayground
- [A-IC-INIT] https://developer.apple.com/documentation/imageplayground/imagecreator/init()
- [A-BCF] https://developer.apple.com/documentation/imageplayground/imagecreator/error/backgroundcreationforbidden
- [A-STY] https://developer.apple.com/documentation/imageplayground/imageplaygroundstyle
- [A-SIZE] https://developer.apple.com/documentation/imageplayground/imageplaygroundoptions/sizespecification-swift.struct
- [A-VTSR] https://developer.apple.com/documentation/videotoolbox/vtsuperresolutionscalerconfiguration
- SDK: `MacOSX.sdk/System/Library/Frameworks/{FoundationModels,ImagePlayground}.framework/…/arm64e-apple-macos.swiftinterface`,
  `VideoToolbox.framework/Headers/VTFrameProcessor_SuperResolutionScaler.h` (Xcode-beta, SDK 27.2)

Microsoft
- [MS-APIS] https://learn.microsoft.com/en-us/windows/ai/apis/ (2026-10-02)
- [MS-PHI] https://learn.microsoft.com/en-us/windows/ai/apis/phi-silica (2026-10-02)
- [MS-GS] https://learn.microsoft.com/en-us/windows/ai/apis/get-started
- [MS-CM] https://learn.microsoft.com/en-us/windows/ai/apis/content-moderation
- [MS-LM] https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.windows.ai.text.languagemodel
- [MS-SJ] https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.windows.ai.text.languagemodel.generatestructuredjsonresponseasync
- [MS-LMO] https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.windows.ai.text.languagemodeloptions
- [MS-RN2] https://learn.microsoft.com/en-us/windows/apps/windows-app-sdk/release-notes/windows-app-sdk-2-0?pivots=stable
- [MS-IMG] https://learn.microsoft.com/en-us/windows/ai/apis/image-generation (2026-10-01)
- [MS-IMGOPT] https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.windows.ai.imaging.imagegenerationoptions
- [MS-IMGSTY] https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.windows.ai.imaging.imagefromtextgenerationstyle
- [MS-ISR] https://learn.microsoft.com/en-us/windows/ai/apis/image-super-resolution
- [MS-ISRAPI] https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.windows.ai.imaging.imagescaler
- [MS-EDGE] https://blogs.windows.com/msedgedev/2026/06/02/expanding-on-device-ai-in-microsoft-edge-new-models-and-apis-for-the-web/
- [MS-FL] https://learn.microsoft.com/en-us/azure/foundry-local/what-is-foundry-local
- [MS-FLCLI] https://learn.microsoft.com/en-us/azure/foundry-local/reference/reference-cli (2026-08-13)
- [MS-FLREST] https://learn.microsoft.com/en-us/azure/foundry-local/reference/reference-rest
- [MS-WML] https://learn.microsoft.com/en-us/windows/ai/new-windows-ml/overview

Linux
- [LX-GNOME] https://discourse.gnome.org/t/local-ai-models-as-shared-desktop-infrastructure/36000

Probe sources (scratchpad, not in the repo): `probe/fm*.swift` (Foundation Models), `probe/sb.swift` + `ProbeFM.app`
(sandboxed background agent), `probe/ip*.swift` (Image Playground), `probe/vt.swift` (VideoToolbox). The composer request
was captured from `target/debug/autopaper compose` against a local capture server, using a temporary data dir.

---

## 7. Test through AutoPaper's real composer (2026-10-09)

- **macOS 27.2, AFM 3 Core Advanced** (a throwaway OpenAI-compatible shim over Foundation Models, run with `autopaper compose` on Must "misty mountains",
  Maybe "lighthouse", Avoid "people"): **4 valid candidates in 39 s, $0**, through the unchanged OpenAI-compatible provider path. **0 of 4 put the Must keyword in
  the prompt** (the composer's own check rejected all four), and one wrote "no people" (the Avoid check also flags a negated mention). Confirms section 1.4: the
  composer needs an on-device profile (repair missing Musts, shorter instructions) before this can be a default writer.
- **Windows 11 ARM64 VM, Foundry Local 0.11.0** (`winget install Microsoft.FoundryLocal`): installs and starts a server on a **dynamic port** (`foundry server status`),
  `GET /v1/models` works (every GPU and CPU variant listed, so a model picker works), and the id to use is the variant id (e.g. `Phi-3-mini-128k-instruct-generic-cpu:3`).
  phi-4-mini only has a GPU variant: it downloaded but **would not load** (no Direct3D 12 GPU in the VM: Dawn error). CPU variants exist for Phi-3 mini, Mistral,
  OLMo, DeepSeek, gpt-oss. Phi-3-mini 128k on the VM CPU: a composer request took 212 s and returned no readable candidate; small direct requests (with and without a
  JSON schema) timed out at 240 s and 600 s. **The VM can't judge Foundry Local's quality or `response_format` support**; that needs a real PC with a GPU or NPU.
- Phi Silica and the other Windows AI APIs were not tried (no NPU in the VM).
