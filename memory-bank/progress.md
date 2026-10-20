# Progress

## Current release: 0.2.0 (2026-10-20)
- Published for Mac, Windows and Linux: https://github.com/msitarzewski/AutoPaper/releases/tag/v0.2.0.
- Live ComfyUI/MOSS discovery, Mac setup wizard and window fixes, experimental Foundry Local; full semantic memory.
- 761 passing core/native tests, both Windows architecture smokes, signed packages and verified update artifacts.
- Details: `tasks/2026-10/261018_moss-image-workflow.md#Final package verification (2026-10-20)`.
- Atmosphere, manual Image Playground, ChatGPT sign-in investigation and sounds remain future work.

## Earlier local batch (historical reboot record)
- **Service availability:** both roles checked before paid work; both Console outcomes recorded, newest usable
  non-disliked selected-mood image returned when unavailable, current preserved when no same-mood image remains.
  Core 465 + CLI 6 pass, 10 existing ignored, strict clippy. Mac 108 tests / 163 cases and Linux 35 / 1 ignored
  plus native builds/VM manual fallback pass; Windows final dual DLL/native build/86 tests/ARM64 smoke pass,
  both real architecture smokes pass; VM suspended again, original profile/desktop untouched.
  Windows GUI unavailable without a logged-in user.
  Signed universal Mac **0.1.1 build 6 installed/opened**, installed/build equality verified. Mac VM stopped, Windows/Scratch restored to original suspension; original profiles/data/background preserved.
- **Native lists/forms:** all platforms inherit native row/control/icon metrics; Mac grouped Forms fit
  short keyword lists; Linux Console collapse/stacked panes resize correctly. All native builds pass;
  Mac 107 tests/162 cases, Windows 83, Linux 34 (+1 existing ignored). Local Mac build 5 was installed, then superseded by build 6,
  strict universal Developer ID signature and installed/build equality verified. Native UI feedback pending.
- **Mac Moods cleanup:** user approved names-only list, detail name, context Use/Delete, no duplicate
  Current/blue icon. Build 4 installed/verified, then superseded by build 5. VM editing/menus/focus verified.
- **Console/budget and charts:** date/run/outcome/request diagnostics, exact prospective budget notice,
  current wallpaper preserved, real outcome/day/actual-model timing charts, help in settings/website.
  Final core 459 tests (10 existing ignored), strict clippy pass. Native builds/tests and applicable VM
  data/export/clear checks pass. Local builds 2/3 superseded by build 5.
- All tasks recorded in `tasks/2026-10/261007_console-budget-transparency.md`; reboot/resume details
  in `activeContext.md#Reboot checkpoint`. Source remains uncommitted on `codex/console-runs`.
- Windows locked visual/physical/native-file-save checks and Linux model-popup/clipboard/full screen-reader
  checks remain open. Public release stays v0.1.0; new packaging/signing/publication deliberately deferred.

## Done (public release history)
- 2026-10-20: **v0.2.0** on all three platforms: live compatible ComfyUI discovery, MOSS-Image, Mac provider setup wizard and window fixes, experimental Foundry Local. Signed/notarized packages and full semantic memory included.
- 2026-10-09: **v0.1.2** on all three platforms: Match my appearance (core setting + prompt line + reporting and toggles on Mac/Windows/Linux), native Mac Moods list with Use / In Use, Dock/menu raise the windows.
- 2026-10-14: **v0.1.5**: security update: `rustls` 0.23.46 (RUSTSEC-2026-0346..0349), tokenizers 0.23, sha2 0.11, `rand` 0.10; CoreSmoke lock covers x64 and arm64; Dependabot has a 7-day cooldown and stays on uniffi 0.31 until uniffi-bindgen-cs supports 0.32.
- 2026-10-13: **v0.1.4**: security round on all three platforms (full report `docs/security-review-2026-10.md`): credentials scrubbed everywhere, owner-only data folder, local-only direct connections, image-decode limit, Sparkle verify-before-extract + signed appcast, pinned CI actions + locked NuGet, Control Flow Guard on the Windows core, new Flatpak signing key (Linux installs re-add the remote), signing VM isolated to `~/vm-exchange`.
- 2026-10-10: **v0.1.3**: Mac "On this Mac" writer (Apple Foundation Models via the core's `SystemModel` port; default on the welcome when available), Console charts, details-first events, JSON/instructions disclosures; `docs/research/built-in-models.md` section 7.
- 2026-10-05: Plan approved (shared Rust core + native SwiftUI / WinUI 3 / GTK4 apps). Repo scaffolded: licence, ignore
  rules, secret scanning config, memory bank, core design (`systemPatterns.md`). Toolchains proven on all three platforms
  (Swift/C# UniFFI round trips; WinUI MSIX sideload; GTK4 on Scratch). Website built (AudioPaper style, red, Lighthouse 100s),
  7 ComfyUI examples, Brew Browser links.
- 2026-10-05: **Core built** (6 module agents → integrator → 3 reviewers → fixer): 375 tests, clippy -D warnings clean, no stubs.
  Real-model calibration: threshold 0.79, echo band 0.72–0.93. `autopaper simulate --days 1100`: 107 echoes, none outside the band.
- 2026-10-06: **Phase 2** apps built, audited and fixed. **Round 3**: moods, performance history + live progress, learned timeouts,
  typed `PaintingFailed`, Demo slow mode (core 442 tests); macOS 22 findings fixed (+ overlay mode, footer, moods dashboard,
  gallery, provenance, reload/stop, Pause keeps wallpaper); Windows 13/13; Linux 21/24. Errors as links; the "working" effect;
  per-model ComfyUI workflows + picker; macOS test VM (tart); Gemini model-list and thinking-level fixes.
- 2026-10-06: macOS **three-column Moods** (sidebar | list | detail, reload/stop in the detail toolbar); website with all pages
  (Help, Shortcuts, Reference, Privacy, Network); PRIVACY.md/NETWORK.md written from the code.
- 2026-10-06: **Gemini fixes** from the user's "failed every recent session" report: painting moved from `/v1` to `/v1beta`
  (Nano Banana 2.1 and preview models only exist there); error redaction no longer hides model names; writer retries now say what
  was wrong and `KeywordNotFollowed` names the keyword + mood (core 445+ tests).
- 2026-10-06: **Windows/Linux parity** with the Mac UI (agents): Moods group, summary + 30-day chart, Grid|Gallery, provenance,
  reload/stop (Ctrl+R/F5), Pause keeps, Windows lock-screen record/restore; Linux sidebar layout. Mac: Sparkle 2.10.0,
  `scripts/release.sh`, KeywordNotFollowed link.
- 2026-10-06: **Published** the repo (first commit, then CI fixes: gitleaks pin, colour codes in rustc's link list, Windows path
  separators, a Swift 6.3 IRGen crash on method-reference Binding setters, a slow-to-type-check expression). CI green on 7 jobs.
- 2026-10-06/07: **Self-hosted updates** on pipx (no Flathub): own signed Flatpak repo, App Installer feed + bundles; scripts
  `pipx-app-updates.sh`, `publish-flatpak.sh`, `publish-windows.sh`, `flatpak-repo-key.sh`; CI `flatpak.yml`.
- 2026-10-07: **Azure Artifact Signing** set up (account, identity validation, PublicTrust profile); Windows bundle signed and
  verified; **v0.1.0 released** on all three platforms; real-key Sparkle update test passed; site shows the real Download buttons.
- 2026-10-08: **v0.1.1 released** on all three platforms (Console, budget transparency, Moods cleanup, local inference timeouts).

## Next
- MOSS-Image, live ComfyUI discovery and the experimental Foundry Local preset are included in the current 0.2.0 batch.
- Deferred from the earlier plan: Atmosphere light/dark + accent (settings off by default; accent applies inside
  the app on macOS/Windows); Image Playground "Paint by hand" (Mac, manual only: the sheet needs a person);
  Sign in with ChatGPT (investigate OpenAI's terms first; drop if not permitted); gentle bundled sounds.
- Foundry Local remains experimental, off by default, pending broader community GPU/NPU reports.
- Smaller: Linux "pick my picture" restore; full VoiceOver/Narrator/Orca passes; optional winget submission; verify Nano Banana 2.1 / Gemini Omni pricing; a `cargo fmt` check needs a one-off reformat (~500 chunks) first.
- Phi Silica waits for Aion Instruct (Windows, from Nov 2026). uuid 1.28 (#4) merges after 2026-10-17.
