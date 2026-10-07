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

### 2026-10-06: Self-hosted updates instead of Flathub
**Status**: Approved (user: "fuck Flathub. We do something else."; hosting on pipx via msitarzewski.com/app-updates)
**Context**: Flathub's requirements forbid AI-generated or AI-assisted manifests (disclosure doesn't exempt them) and
AI-opened PRs; GitHub Pages can't hold the ~130 MB embedding model as a single file, and App Installer's handling of
GitHub's release redirects is undocumented.
**Decision**: Linux ships from our own GPG-signed Flatpak repository at https://msitarzewski.com/app-updates/autopaper/flatpak
(one-click `AutoPaper.flatpakref`; the GNOME runtime still comes from Flathub's runtime repo), plus a `.flatpak` on
GitHub Releases that points at the same repository. Windows' App Installer file and MSIX bundles are hosted there too
(winget keeps GitHub Release URLs). CI builds the Flatpak bundles (x86_64 + aarch64); publishing (signing, rsync to
pipx) runs on the Mac. Server setup: `scripts/pipx-app-updates.sh`, following ~/Clean/pipx/DEPLOYING.md (Michael runs
it; only msitarzewski.com's slice).
**Consequences**: no discovery through Flathub's catalogue; updates still come through Flatpak/GNOME Software/Discover;
one more host (msitarzewski.com, no access logs) in NETWORK.md/PRIVACY.md; the repo signing key must be backed up.
