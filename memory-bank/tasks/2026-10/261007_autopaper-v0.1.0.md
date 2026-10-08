# 261007_autopaper-v0.1.0

## Objective
Build the "Desktop Wallpaper Agent" (a Reddit post) natively for macOS, Windows and Linux, accessible, with users' own API keys, and
ship it as an open-source project the way AudioPaper was shipped.

## Outcome
- Shipped v0.1.0 on all three platforms: https://github.com/msitarzewski/AutoPaper/releases/tag/v0.1.0 (site
  https://msitarzewski.github.io/AutoPaper/). Core: ~445 tests, clippy clean; macOS 100 tests; Windows 76; Linux 29; CI green
  (7 jobs: secret scan, core on 3 OSes, 3 app builds).
- Real-device checks in VMs: macOS Gatekeeper + Sparkle update (real key), Windows install/update from the real server with a
  Microsoft-chain signature and no extra trust, Linux Flatpak install from the real repository with GPG verification.

## What was built
- `core/` shared Rust core (composer, memory with local embeddings, echoes, taste, moods, schedule/budget, performance history,
  SQLite store, providers OpenAI/Gemini/Ollama/OpenAI-compatible/ComfyUI/Demo, one HTTP client, UniFFI).
- `apps/macos` (SwiftUI/AppKit, Sparkle), `apps/windows` (WinUI 3, MSIX/App Installer), `apps/linux` (GTK4/libadwaita, Flatpak).
- `site/`, PRIVACY.md, NETWORK.md, README/SECURITY/CONTRIBUTING, CI workflows, release and publish scripts.

## Patterns applied / discovered (see `systemPatterns.md`, `techContext.md`)
- One problem said once as a link to the fix; Pause keeps the wallpaper; overlay mode on macOS; per-model ComfyUI workflows at the largest
  supported size; performance history for progress and learned timeouts; typed errors (`PaintingFailed`, `KeywordNotFollowed`).
- Release infrastructure: self-hosted feeds on pipx behind Caddy (no Flathub), Azure Artifact Signing, dedicated Flatpak GPG key.
- Gotchas worth remembering: a `$([[ … ]] && echo …)` under `set -e` exits the script; CARGO_TERM_COLOR=always puts escape codes in
  rustc's note; Xcode 26.6's compiler crashes on method-reference Binding setters and times out on one-expression closures; the
  gitleaks action's default version can't read `[[allowlists]]`; macOS's openrsync ignores `--chmod`.

## Architectural decisions
See `decisions.md` (Rust core + native UIs, local embeddings, overlay default, per-model ComfyUI + largest size + performance history,
Moods, errors as links, VM-only GUI testing, retries that say what was wrong, Windows lock-screen capabilities, Artifact Signing,
self-hosted updates instead of Flathub, release order, Qwen examples).

## Artifacts
- Release: https://github.com/msitarzewski/AutoPaper/releases/tag/v0.1.0 · Update feeds: `site/static/appcast.xml`,
  https://msitarzewski.com/app-updates/autopaper/ (AutoPaper.appinstaller, AutoPaper.flatpakref, flatpak/, windows/).

## Post-release checkpoint (2026-10-07; recorded before reboot)
The v0.1.0 release and its QA above remain historical. Subsequent Console/budget transparency, Console
charts/help, Mac Moods cleanup, native list/form tasks and service preflight/selected-mood fallback are recorded in
[261007_console-budget-transparency.md](261007_console-budget-transparency.md).
Current local Mac is 0.1.1 build 6; public feeds remain v0.1.0. These follow-ups are uncommitted on
`codex/console-runs`; the user explicitly deferred public releases to gather a larger batch.
See `../../activeContext.md#Reboot checkpoint` for exact installed hash, latest QA, open checks and resume steps.
