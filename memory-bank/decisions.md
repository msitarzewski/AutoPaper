# Decisions

### 2026-10-05: Shared Rust core + native UIs
**Status**: Approved (user chose it from three options)
**Context**: Native apps on macOS, Windows and Linux; the agent's memory/novelty/echo logic must behave identically.
**Decision**: One Rust crate (`autopaper-core`) holds every decision; UniFFI exposes it to Swift (XCFramework) and C#
(uniffi-bindgen-cs); the GTK app uses it directly. Hosts own UI, timers, OS integration and secure key storage.
**Alternatives**: three separate codebases (logic written three times, drift); Mac first in Swift (Windows/Linux later).
**Consequences**: UniFFI pinned to 0.31.2 until uniffi-bindgen-cs supports 0.32; FFI changes ripple to three apps, so
they're batched per round; generated Swift compiles in Swift 5 mode.

### 2026-10-05: Local embeddings for memory
**Status**: Approved (within the core design)
**Decision**: bge-small-en-v1.5 via candle, bundled with each app; HashingEmbedder fallback. Calibrated: repeat ≥ 0.79,
echo band 0.72–0.93. **Why**: offline, identical on every platform, no extra API calls or hosts.

### 2026-10-06: Desktop overlay by default on macOS
**Status**: Approved (user)
**Decision**: AudioPaper's DesktopOverlay (click-through window per display at desktop level + 1) is the default
"Over my wallpaper" mode; "As my wallpaper" (setDesktopImageURL) is optional for the lock screen/Mission Control.
**Why**: the real wallpaper (Aerials) is never touched; quitting uncovers it. Windows/Linux have no supported
desktop-layer window, so they set the real wallpaper and restore the person's own (and the lock screen, Windows).

### 2026-10-06: Per-model ComfyUI workflows, largest supported size, performance history
**Status**: Approved (user)
**Decision**: bundled tested workflows per model (Z-Image Turbo default, Krea 2 Turbo, Qwen-Image 2.1), offered only
when installed; paint at each model's largest supported size (Apple-silicon 8×-VAE cap 2,965,760 px); record every
provider call's timing locally → progress, time left, learned timeouts (stall/30-min ceiling), estimates, early starts.

### 2026-10-06: Moods
**Status**: Approved (user)
**Decision**: named keyword sets with their own Surprise; one active; novelty and taste stay global; echoes prefer the
active mood. Existing keywords migrate into the first mood.

### 2026-10-06: Errors as links
**Status**: Approved (user)
**Decision**: a problem with a setting is said once per view, as a link/button to the fix; dependent controls are
disabled until fixed; the view refreshes itself when it is. (app-spec rule 6a)

### 2026-10-06: GUI testing in VMs only
**Status**: Approved (user)
**Decision**: macOS GUI checks run in a headless tart VM (`scripts/macvm`); Windows/Linux in their Parallels VMs.
Nothing launches on the user's desktop; only `scripts/macvm` is shared into the macOS VM (never `.env`).

### 2026-10-06: How v0.1.0 ships
**Status**: Approved (user)
**Decision**: All three platforms in one release from a public GitHub repo (msitarzewski/AutoPaper). macOS as AudioPaper
(notarized DMG + Sparkle, appcast on GitHub Pages); Windows signed with Azure Artifact Signing (MSIX/msixbundle on
GitHub Releases, winget, App Installer update feed on Pages); Linux as a Flatpak on Flathub plus a bundle on GitHub.
**Alternatives**: Mac first, Windows/Linux later; Microsoft Store signing (free, but no GitHub download and per-release
review); GitHub-only Flatpak (no automatic updates).
**Consequences**: Windows/Linux parity and packaging come before the release; Windows signing waits on the user's
Artifact Signing identity check; one Artifact Signing profile can sign any of the user's Windows apps.
