# 2026-10-18 — MOSS-Image as a fourth bundled ComfyUI workflow

**Status: approved for release as 0.2.0 on 2026-10-20; final packaging and cross-platform QA in progress.**
The original MOSS work below was extended by the following batch in `codex/comfy-discovery`.
The user asked: "Will you please add this to my options for the autopaper workflow in comfy?" (MOSS-Image,
github.com/OpenMOSS/MOSS-Image). Main and the installed app (0.1.1 build 6) are untouched.

## Why it can be a bundled workflow now

- Bundled workflows use stock nodes only (`every_bundled_workflow_…`: `custom_nodes` empty). OpenMOSS's own ComfyUI
  plugin is a custom node, so it can't be one.
- ComfyUI master gained **native** MOSS-Image on 2026-10-16 (PR #17095; fused SiTU #17099): `comfy/ldm/moss_image`,
  `CLIPLoader type "moss_image"` (Qwen3.5-9B), `EmptyFlux2LatentImage`. Comfy-Org published the repack
  `Comfy-Org/MOSS-Image` and the template `image_moss_image_t2i` (workflow_templates 0.11.89).

## The workflow (`core/resources/comfyui/moss-image-t2i.json` + `.map.json`)

- Comfy-Org's template in API format, with the same node ids as the others (4 prompt, 7 latent, 8 KSampler,
  10 SaveImage):
  - `UNETLoader moss_image_sft_int8_convrot` (5.41 GB)
  - `CLIPLoader qwen3.5_9b_w4a8_gptq` type `moss_image` (7.34 GB)
  - `VAELoader flux2-vae_small` (0.25 GB)
  - KSampler 25 steps, cfg 2.5, `er_sde` / `simple`
  - negative = the user guide's `nsfw, worst quality, bad anatomy, watermark, jpeg artifact`
- **The template's two switches, as shipped:** skip-layer guidance **off** (`switch_1` false sends the
  UNETLoader's model straight to the KSampler; verified from the link table, `ComfySwitchNode` on_false). The
  prompt upsampler (`TextGenerate` on the same encoder) is on in the template, but left out here: AutoPaper's
  writer already composes a full caption, and the upsampler would add a second, slow text-generation pass.
- Licence Apache-2.0 (code and weights).

## Sizes — measured, not assumed

The FLUX.2 VAE fails like the 8× VAEs on Apple silicon. On the M5 Max with ComfyUI master 872eb6a3, decode-only
graphs (`EmptyFlux2LatentImage` → `VAEDecode`):

| size | result |
|---|---|
| 2048×1440 | decodes |
| 2048×1456 | `MPSGraph does not support tensor dims larger than INT_MAX` |
| 2048×2048 | the same error |

So the `MODEL_LIMITS` entry is `moss_image` → **2048 a side, `MPS_VAE8_MAX_PIXELS` (2,965,760)**. The user guide
allows up to 4 MP, so this is stepped down. A 16:10 Retina hint gives 2032×1312, as with Z-Image Turbo.

## Verified on the Mac

- **Recorded request** `core/tests/fixtures/comfyui/prompt-moss-image.request.json`: the template filled with the
  prompt AutoPaper's writer composed for "Drowned Root Glow" (2026-10-18 11:12), 2032×1312, seed
  6417253117930451, posted as-is to ComfyUI. It succeeded in **446 s** cold (16.6–17.3 s a step) and the render
  matches the prompt (viewed). `filling_each_bundled_template_reproduces_the_graph_verified_on_the_mac` compares
  against it.
- **Through the engine:** the CLI, with a scratch `--data-dir`, Demo writing, ComfyUI painting,
  `image_provider.model = moss_image_sft_int8_convrot.safetensors`, `--display 4112x2658`.
  - `models images` against the live server listed all four: Z-Image Turbo (default), Krea 2 Turbo, MOSS-Image,
    Qwen-Image 2.1.
  - `generate` → "Golden hour beyond Island Archipelago", 2032×1312, **405 s** warm, with step progress and
    time-left estimates. The style was followed (vector illustration, lavender/emerald/olive; viewed).
- **Fixture `object_info.response.json` extended from the live server** (master 872eb6a3):
  - the `EmptyFlux2LatentImage` class in the same trimmed shape
  - `moss_image` in `CLIPLoader.type`
  - the three files among the loaders' choices, each placed where the live server lists it

  A JSON round trip reproduced the old file byte for byte before the merge, so the diff is only the additions.

## Code and docs touched

- `core/src/providers/comfyui.rs`:
  - module doc
  - `MODEL_LIMITS` (+ its source note)
  - `BUNDLED_SOURCES` [;4]
  - tests:
    - the bundled list
    - the recorded-graph comparison
    - `missing` without `moss_image` / `EmptyFlux2LatentImage`
    - `list_models` order and the `fewer` case
    - limits by file name
- Name tables: macOS `BundledWorkflows.names`, Linux `KNOWN_MODELS` [;11]. Windows reads the embedded
  `*.map.json`, so nothing changes there.
- `README.md`, `docs/app-spec.md`, `THIRD-PARTY-NOTICES.md`, `site/pages/{help,reference,index}.html` (help: the
  files, Apache 2.0, "about seven and a half minutes", needs an October 2026+ ComfyUI), `systemPatterns.md`
  (limits row, licence paragraph).

## Tests

- Core: **419 + 74 + 1 pass** (8 + 2 ignored), strict clippy (`-D warnings`) clean.
- macOS:
  - `scripts/build-xcframework.sh --smoke` passes. The smoke executable's `invalid input: displaySizeInvalid`
    is its deliberate error-path check.
  - App tests (CI's unsigned `xcodebuild test`, DerivedData `build/ci` in the worktree): **117 tests in 23
    suites pass**. That includes the test that every bundled map's model is in `BundledWorkflows.names`.
- Not run here: the Linux app tests (need GTK 4.22, the CI container) and Windows (VM).

## Follow-up: discovery, Mac setup and window fixes (approved 2026-10-20)

The user approved the final Now fix and then requested “Ship the current work.” This authorizes the current
batch's commits, release packages, feeds and site publication. Existing planned version 0.2.0, Mac build 11.

- `core/src/providers/comfyui.rs:634` discovers installed compatible models, binds exact installed filenames,
  and adapts official server templates using `core/src/providers/comfyui/templates.rs`. It validates nodes,
  dependencies and inputs; omits ambiguous or unsupported graphs; keeps model-specific sampling; uses conservative
  dimensions. Bundled tested presets win. Older servers retain bundled discovery.
- `core/src/engine.rs:2019` resolves the actual model before writing, so a missing model never spends on an idea.
  Mac, Windows and Linux model menus explain Automatic, missing and empty choices.
- Mac `WelcomeView.swift` reuses `ProviderSection`: keywords → provider route → actual connection/model setup,
  with a read-only check before finishing. Apple Intelligence still needs an explicit painter. Demo is opt-in;
  skipping, interruption or Stop during the first run keeps the schedule unarmed.
- Mac `NowView.swift` uses an aligned, scrollable image/caption area and fixed status footer. Its footer has a
  340pt minimum width to prevent a wrapping narrow-keywords notice from expanding the entire window content
  above its frame. Reproduced the old bug with retained profile data; the fix recovers that same profile on launch.
- `SettingsView.swift` caps pane height to the display and scrolls long forms. `AutoPaperApp.swift`,
  `AppDelegate.swift`, `AppCommands.swift` capture and activate the actual Settings window on explicit requests.
  Passive model/pane changes do not steal focus.

### Validation before packaging

- Core + CLI: 515 tests passed, 12 existing ignored; strict clippy passed.
- Mac: 123 tests in 24 suites passed. VM verified wizard setup, missing ComfyUI, interruption/skip/Stop,
  first-run scheduling, Settings scroll and activation, light/dark and minimum display/window sizes.
- Live ComfyUI discovery found five compatible models. A discovered Qwen Turbo workflow produced a real
  768×432 image in 20.7 seconds. The user also generated a real MOSS wallpaper in the preview app.
- Now regression: old source reproduced 210×1483 sidebar inside a 1040×780 window; fixed source recovered
  the same profile to 210×687 without resetting data. Further generation, relaunch and minimum-window checks passed.
- Final cross-platform package results are recorded below when complete. Host GUI tests were not run.

### Release dependency scan (2026-10-20)

OSV scanned Cargo.lock and the Windows app lockfile. Four maintenance notices remain: paste, zbus_names,
zvariant and zvariant_derive. Three new zip 8.6.0 extraction advisories (RUSTSEC-2026-0367/0368/0369) were
reviewed against the locked dependency source: zip is transitive through Candle 0.11; Candle only reads named
archive entries for optional NumPy/Pickle paths and never calls ZipArchive::extract or ZipStreamReader::extract.
AutoPaper's `core/src/embed.rs` loads its bundled safetensors model, not those archive paths. The affected
extraction APIs are not reachable from this app. This is an applicability assessment, not a clean scan claim.
Track Candle's eventual zip9 migration and the maintained D-Bus replacements; dependency PRs remain separate.

