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
**Status**: Approved (user); the Linux/Windows hosting parts were superseded the same day by "Self-hosted updates instead of Flathub" below
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

### 2026-10-06: Writer retries say what was wrong; a typed "keywords not followed" error
**Status**: Approved (user report: "something isn't right with memory or the google models… failed every recent session")
**Context**: The logs showed two separate failures: Gemini painting 404s (`/v1` vs `/v1beta`, fixed) and "none of the text model's
ideas followed the keywords", where retries never told the model what it missed.
**Decision**: `ComposeContext.corrections` feeds the most common problems back on retries; when a Must is left out or an Avoid used
every time, the core returns `AutoPaperError::KeywordNotFollowed { keyword, weight, mood_id }`; every app says it in one line naming
the keyword, as a link that opens that mood (rule 6a). Painting for Google uses `/v1beta`, like its model list. `net::redact` no
longer hides model names that mix letters and digits.

### 2026-10-06: Windows lock screen restore needs two capabilities
**Status**: Approved (user: "Yes"); implemented by the Windows agent
**Decision**: Windows records the person's lock screen picture before the first change and restores it on Quit/Restore (never
overwriting a picture they chose since; Spotlight/slideshow can't be re-enabled by an app, so it says so). The manifest adds
`picturesLibrary` (Windows only hands an app the lock screen picture with it) and `RegistryWriteVirtualization disabled` with the
restricted capability `unvirtualizedResources` (otherwise Windows' own lock-screen code writes to AutoPaper's private registry copy
and Settings keeps showing Spotlight). Both are explained in PRIVACY.md and shown in App Installer's dialog.

### 2026-10-07: Windows signing with Azure Artifact Signing; Mac and Linux are signed with the user's own keys
**Status**: Approved (user), done
**Decision**: Individual identity validation (US) → PublicTrust profile `autopaper` on account `msitarzewski-signing` (East US,
Basic $9.99/mo); SignTool + the Artifact Signing dlib, timestamped. Certificates last 3 days and renew; signatures stay valid.
Microsoft's documented validation time is 1–20 business days (it took hours here). The same account can sign the user's other
Windows apps. Mac: Developer ID + notarization + Sparkle EdDSA key ("AutoPaper", login Keychain). Linux: a dedicated GPG key for the
Flatpak repository (`~/.config/autopaper/flatpak-gnupg`, no passphrase, public key committed).

### 2026-10-07: Release order and "all three together" relaxed, then restored
**Status**: Approved (user)
**Context**: The identity check was thought to take days, so the user chose to ship Mac and Linux first; it finished in hours, so the
user asked to go live once all three were ready. **Decision / order**: build and verify each platform, publish Windows and Linux files
to the server, create the GitHub release, then commit the site's Download buttons and the Sparkle appcast LAST so nothing points at a
file that isn't there yet.

### 2026-10-07: Qwen-Image 2.1 examples stay on the site
**Status**: Approved (user: "This is NOT true. Please verify. Leave them as is.")
**Context**: The repo scan noted the Qwen Research License (non-commercial = "research or evaluation purposes only"; outputs'
ownership isn't addressed). The user says the Qwen team clarified it. Defaults stay permissive (Z-Image Turbo, Apache-2.0). Link the
clarification in the Credits page if the user provides it.

### 2026-10-07: Console run capture and budget preserves the current wallpaper
**Status**: Approved implementation; user requested installation after QA
**Decision**: Extend Engine/Store with local run records and capture actual provider POST payloads at
the existing HTTP boundary. Native app views group dates and runs. Keep 200 runs / 30 days with bounded,
labelled traces; omit authentication headers and image bytes; clear independently of wallpaper memory.
**User requirement**: A prospective monthly budget block explains spent/limit/next estimate and leaves
the latest displayed wallpaper in place, including when provider-failure fallback is RevisitLiked.
**References**: `tasks/2026-10/261007_console-budget-transparency.md`; `systemPatterns.md#Console run records`

### 2026-10-07: Console statistics retain exact run/model provenance
**Status**: Implemented and verified within the authorized Console refinement
**Decision**: Keep request-estimate keys unchanged, link only new timings to their exact run IDs (migration 6),
and store actual response models separately (migration 7). Aggregate retained outcomes/UTC days and exact
linked provider/job/model call times. Do not invent old run/timing associations. Blocks/cancellation/interruption
stay separate from success/failure rate and average finished-run duration.
**References**: `core/src/engine.rs:342`, `core/src/store.rs:722`; follow-up task record and systemPatterns.

### 2026-10-07: Native list/form defaults and clean Mac Moods
**Status**: User-requested requirements implemented/verified; earlier Moods cleanup approved, latest UI feedback pending
**Decision**: Use each toolkit's contextual native list/control/icon sizing, typography, selection and form grouping;
remove redundant app-drawn rules/header bands. Keep meaningful content/actions/AX and functional photo/pane geometry.
Mac mood list shows names only; editable name lives in detail; Use/Delete are context actions with native confirmation.
**References**: `docs/app-spec.md:8`, `apps/macos/AutoPaper/Views/MoodsView.swift:140`, follow-up task record.

### 2026-10-07: Substantial release batches; local builds are separate
**Status**: Explicit user decision
**Decision**: Gather meaningful changes before publishing/signing a public release. Local builds/installations continue;
0.1.1 build 5 is installed on Mac, while public feeds remain v0.1.0. Windows signing readiness persists for the final
batch, but no cached/stale 0.1.1 package may be reused. Reboot documentation does not authorize commit/push/publication.
**References**: `tasks/2026-10/261007_console-budget-transparency.md`, `activeContext.md#Reboot checkpoint`.

### 2026-10-07: Check both services before generation; revisit the selected mood
**Status**: User-requested behavior implemented and verified; Mac build 6 installed
**Decision**: Perform independent read-only writing/painting availability checks in parallel after the budget gate,
with an 8-second limit and cancellation. Record both results in Console. Validate the local workflow afterward,
before any paid call. Use existing model-list endpoints and ComfyUI `/object_info`, including custom workflows.
If a check fails, use the newest usable, non-disliked saved image from the active mood captured at run start,
regardless of generic scheduled fallback preference or echo source's mood. Skip missing/corrupt originals;
when none remain, keep current and return the error. A saved fallback stays a failed Console attempt and
never produces a new-wallpaper notification. Preserve ordinary scheduled backoff and strict core APIs;
native manual/echo actions consume the new Shown-returning APIs.
**References**: `systemPatterns.md#Service availability before wallpaper generation`, `core/src/engine.rs:2021`;
`core/tests/engine.rs:2393`.

### 2026-10-08: Local servers wait 10 minutes, and longer when their history says so (no timeout setting)
**Status**: Approved (user: "make it up to 10 minutes… preloading can take a while from scratch… or an input field? thoughts?" → "build 1 to 3")
**Context**: Ollama and OpenAI-compatible text had a fixed 180 s and compat painting 300 s; a model loaded from scratch can exceed that.
**Decision**: `providers::local_timeout_secs(expected)` = max(10 min, 3 × the learned estimate), at most an hour, for Ollama text and
OpenAI-compatible text and images; `ComposeRequest.expected_secs` (new) is filled by the engine from the Concepts timings, as
`ImageRequest.expected_secs` already is. Hosted services keep 60 s/240 s; ComfyUI keeps its stall windows and 30-minute ceiling.
**Alternatives**: a per-provider input field (three apps + the settings format, and nobody can guess the number); a "loading the model"
progress stage (needs an FFI enum change and text in all three apps — left for a UI round); background warm-up (not built).
**Consequences**: compat servers that are remote hosts also wait up to 10 minutes (Stop works at any time); core-only change, ships with the next release.
