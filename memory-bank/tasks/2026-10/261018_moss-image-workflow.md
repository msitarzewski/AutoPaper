# 2026-10-18 — MOSS-Image as a fourth bundled ComfyUI workflow

**Status: released as [0.2.0](https://github.com/msitarzewski/AutoPaper/releases/tag/v0.2.0) on 2026-10-20.**
Publication and final CI closure were verified on October 21; see [Release closure](#release-closure-verified-2026-10-21).
The original MOSS work below was extended by the following batch in `codex/comfy-discovery`.
The user asked: "Will you please add this to my options for the autopaper workflow in comfy?" (MOSS-Image,
github.com/OpenMOSS/MOSS-Image). The original branch record below predates the release follow-up; the host installed app/profile were not changed during release QA.

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
- Completed cross-platform package results are recorded below. Host GUI tests were not run.

### Release dependency scan (2026-10-20)

OSV scanned Cargo.lock and the Windows app lockfile. Four maintenance notices remain: paste, zbus_names,
zvariant and zvariant_derive. Three new zip 8.6.0 extraction advisories (RUSTSEC-2026-0367/0368/0369) were
reviewed against the locked dependency source: zip is transitive through Candle 0.11; Candle only reads named
archive entries for optional NumPy/Pickle paths and never calls ZipArchive::extract or ZipStreamReader::extract.
AutoPaper's `core/src/embed.rs` loads its bundled safetensors model, not those archive paths. The affected
extraction APIs are not reachable from this app. This is an applicability assessment, not a clean scan claim.
Track Candle's eventual zip9 migration and the maintained D-Bus replacements; dependency PRs remain separate.


### Final package verification (2026-10-20)

- Frozen application source: commit `2f8b79e`, tag `v0.2.0`.
- Core + CLI: 515 passed/12 existing ignored; strict clippy clean. Initial sandbox run denied localhost test-server
  binds; the authorized network-enabled rerun passed without source changes.
- Mac: 123 tests/24 suites; universal Release build; all signatures/entitlements, exact bundled BGE model,
  Apple notarization/stapling and Sparkle archive/feed signing passed. Production VM executable matches
  `faeecd914268c2b9367ef137462178bd4e7a219ef6648ab02e14d3843edc36bc`.
- Exact Mac production bundle: Full memory, four real Demo generations, first-run setup, Settings scrolling/front,
  narrow notice at 860×592 on 1280×800, History→Now and same-profile relaunch passed. Guest data only; no host GUI tests.
- Linux: 37 tests including normally opt-in Secret Service test passed under an isolated keyring; native build
  and strict clippy passed. Real installed-model popup includes MOSS-Image and discovered Qwen Turbo. Other
  GUI/error-state checks and selection persistence were not exercised; automated model-state coverage passed.
  Eight nonfatal GTK assertions appeared at startup; no crash or menu issue was observed, cause uninvestigated.
  Original process resumed; all 300 profile files, preferences and background settings/bytes match the baseline.
  Temporary profiles/services removed; Scratch remains in its original running state.
- All 7 CI jobs and both tagged Flatpak builds plus build attestations passed.
- Windows tests found one old contract assertion expecting a hard-coded ComfyUI default. Follow-up commit
  `df7cc19` corrects it to the empty automatic/discovered default. Application source stays exactly at the release
  tag; this test-only correction does not alter any packaged runtime bytes or Flatpak provenance.
- Windows native tests: 86 passed, none skipped. GUI verified all five installed choices, MOSS selection/refresh,
  an unavailable saved model and an offline server. Original profile/preferences hashes and wallpaper match the
  baseline; QA app and shared metadata relay stopped. ARM64 and x64 real runtime smokes pass with full BGE memory.
  The VM lacks a global x64 .NET 10 runtime, so the x64 smoke used a temporary self-contained publish with locked
  restore, as the actual release does. No global runtime installation or production-source change.
- Signed Linux downloads and update repository published. A fresh container client imported only the public
  repository key and successfully verified the HTTPS summary and both signed release commits:
  ARM64 `2a6e1bafec1f5b4fcdbb8738a701302ddb15c3dfcbef616c31c3b013d06472e2`;
  x64 `e3a5d2d0e8ac80dc28b6994e1fd2f62a5eaf2e724fbbf86e74cfa7013796ffcb`.
- Site: 8 pages, 219 local links/anchors pass; review-patch secret scan clean.
- Windows production signatures on both architecture packages and the combined bundle are Valid, timestamped,
  and pass SignTool /pa /v with zero warnings/errors. Both exact core DLLs and six bundled model-file hashes pass.
  Combined bundle SHA256: `0a82be5ded54d009c3f86fb387826bc5bd282deb31e43960314e65b305ab5aa9`.
- Windows feed and bundle published; HTTPS types, no-cache feed, content length, server checksum and 206 range
  response verified. Existing bundles retained. New 0.2.0 winget manifests generated by the existing release
  script; earlier version manifests retained, no winget registry submission.
- Public GitHub release contains nine verified assets; every uploaded size/SHA-256 matches the signed local file.
  Release: https://github.com/msitarzewski/AutoPaper/releases/tag/v0.2.0.
  Signed Mac feed and updated site were published by the successful Pages run at release metadata commit `f8f668b`;
  live publication checks are recorded below.
- Mac task VM stopped, Linux original process resumed and profile/background restored, Windows profile/preferences/
  wallpaper restored. Windows 11 returned to its original suspended state. No host GUI tests.

### Release closure (verified 2026-10-21)

The public release was published at **2026-10-20 06:34:43 UTC** with nine assets. Versions are macOS
**0.2.0 build 11**, Windows **0.2.0.0**, and Linux **0.2.0**. No package was rebuilt or tag moved for the
subsequent test correction or documentation reconciliation.

| Source / publication step | Commit | Result |
|---|---|---|
| Frozen runtime source, immutable `v0.2.0` | [`2f8b79e`](https://github.com/msitarzewski/AutoPaper/commit/2f8b79ec958f2a26117d6b5719214d9c726b9793) | Signed packages built from this source |
| Windows test-only default-model correction | [`df7cc19`](https://github.com/msitarzewski/AutoPaper/commit/df7cc19) | Expected automatic/discovered default; no runtime-byte change |
| Published feed, site and release metadata | [`f8f668b`](https://github.com/msitarzewski/AutoPaper/commit/f8f668b1b9011c2aa825c7fb0bcf4d83f4e7d92d) | Main publication commit |

| Workflow | Source | Verified result |
|---|---|---|
| [Pre-release CI 39377415389](https://github.com/msitarzewski/AutoPaper/actions/runs/39377415389) | `2f8b79e` | All seven jobs passed |
| [Flatpak 39378287021](https://github.com/msitarzewski/AutoPaper/actions/runs/39378287021) | `v0.2.0` / `2f8b79e` | Both architecture builds and attestations passed |
| [Pages 39381286459](https://github.com/msitarzewski/AutoPaper/actions/runs/39381286459) | `f8f668b` | Build and deployment passed |
| [Final release CI 39381286539](https://github.com/msitarzewski/AutoPaper/actions/runs/39381286539) | `f8f668b` | All seven jobs passed; no remaining release CI wait |

The seven CI jobs cover core tests on macOS/Windows/Ubuntu, the unsigned Mac app, Windows WinUI x64,
Linux GTK and secret scanning. The **761 passing tests** above also include native VM suites and the
normally opt-in Linux Secret Service test; this total is not a claim that CI alone exercised all manual QA.

#### Published assets

All nine uploaded asset sizes and SHA-256 digests matched the local release files. The primary downloads
and feed are recorded here; the release also contains `AutoPaper-0.2.0.sha256` and the two architecture-specific
Flatpak `.sha256` files. The GitHub release is the durable download source; temporary build directories are
local evidence caches and should not be required to resume work.

| Asset | Bytes | SHA-256 |
|---|---:|---|
| [macOS DMG](https://github.com/msitarzewski/AutoPaper/releases/download/v0.2.0/AutoPaper-0.2.0.dmg) | 104636451 | `452f4c1af8e1310386e142e61a4c235d4654d6f567735e1782a852bc2f992104` |
| [Sparkle ZIP](https://github.com/msitarzewski/AutoPaper/releases/download/v0.2.0/AutoPaper-0.2.0.zip) | 96286335 | `b5a0c7ecfda4b1242210e5ed54bf1bc115073a586e771de7c6e6ba60ee1ddc31` |
| [Windows x64 + ARM64 bundle](https://github.com/msitarzewski/AutoPaper/releases/download/v0.2.0/AutoPaper_0.2.0_x64_arm64.msixbundle) | 392332397 | `0a82be5ded54d009c3f86fb387826bc5bd282deb31e43960314e65b305ab5aa9` |
| [Windows App Installer feed](https://github.com/msitarzewski/AutoPaper/releases/download/v0.2.0/AutoPaper.appinstaller) | 696 | `c024c303e63df10a3ccebe88c43dfd6bb4aa67938e557998b7fa703269976e60` |
| [Linux aarch64 Flatpak](https://github.com/msitarzewski/AutoPaper/releases/download/v0.2.0/AutoPaper-0.2.0-aarch64.flatpak) | 66697376 | `1468f7aa3d276d75f838ce827d00a42422029b51b491daf2bdfc52c6b1834551` |
| [Linux x86_64 Flatpak](https://github.com/msitarzewski/AutoPaper/releases/download/v0.2.0/AutoPaper-0.2.0-x86_64.flatpak) | 67579696 | `0b791da0d6f74029bf264636e66e1612f870637ada90c57b80757537b70220d4` |

#### Public distribution checks

- [Website](https://msitarzewski.github.io/AutoPaper/) displays 0.2.0.
  [GitHub latest](https://github.com/msitarzewski/AutoPaper/releases/latest) resolves to `v0.2.0`.
- [Mac appcast](https://msitarzewski.github.io/AutoPaper/appcast.xml) is byte-identical to the signed release
  feed and advertises build 11, the correct ZIP URL and 96286335-byte enclosure; that archive returns HTTP 200.
- [Windows feed](https://msitarzewski.com/app-updates/autopaper/AutoPaper.appinstaller) and its bundle were
  checked for HTTPS, correct content types, no-cache feed, content length, SHA-256 and HTTP 206 range support.
- [Flatpak repository](https://msitarzewski.com/app-updates/autopaper/flatpak/) and
  [install reference](https://msitarzewski.com/app-updates/autopaper/AutoPaper.flatpakref) are published.
  A fresh client with only the public key verified the summary and the two commits recorded above. Current
  signing fingerprint: `F4562E1A6706E16CFBFA5DAEF91A07754E829612` (rotated October 13).
- The production model file matched SHA-256
  `3c9f31665447c8911517620762200d2245a2518d6e7208acc78cd9db317e21ad` across platforms. Packaged Windows
  core DLLs and loaded smoke-test DLLs matched: ARM64
  `8736b85350400acc0cbf464f4be128e3c905cf51b115ff5e10d7747e2b3b3bc5`, x64
  `637387776f6c821c0356da3214e4e5690ceb96020aa82fc7717fd63099265b4f`.

#### Remaining work and scope limits

- Linux empty/missing/offline model-picker GUI states and selection persistence were not exercised. The eight
  nonfatal GTK startup assertions need investigation; automated coverage does not close those manual checks.
- Full VoiceOver/Narrator/Orca passes remain open. Foundry Local remains experimental and off by default,
  with broader GPU/NPU verification pending community reports.
- Dependency PRs #4 (uuid), #6 (base64), #7 (sha2) and #8 (serde_json) were still open on October 21. Review
  them separately, together with the maintenance notices and upstream zip migration noted in the scan assessment.
- The generated 0.2.0 winget manifests have not been submitted to the registry. No Mac App Store submission
  or approval is implied by this direct-distribution release.
- Host app/profile were not changed during release QA. VM cleanup described above is the release-time state;
  rediscover current VM/process state before any future work.

The October 21 documentation follow-up reuses this task record and the existing Memory Bank, public docs,
security assessment and website sources. `scripts/third_party_notices.py` refreshed the existing generated
notices from the unchanged release lockfile (385 crates). It reconciles release status and pending work without changing the
signed packages, release tag or appcast.

Documentation validation on October 21: the existing site renderer built all eight pages; 240 local site
links/resources and unique IDs passed. All 238 local links/resources in the 20 changed Markdown documents
resolved, including anchors. Generated notices passed `third_party_notices.py --check`; `git diff --check`
and the redacted diff secret scan passed. This documentation-only follow-up does not rerun or replace the
761-test release QA above.
