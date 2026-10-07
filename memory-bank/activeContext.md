# Active Context

**State** (2026-10-06, saved before a compaction): **BUILD**, after round 3. Branch `build/v1`, **no commits ever**
(global rule: commit only when the user asks). Spec for every app: `docs/app-spec.md`; design: `systemPatterns.md`;
toolchains/VMs: `techContext.md`; history: `progress.md`, `tasks/2026-10/README.md`; decisions: `decisions.md`.

## Done since the 2026-10-06 compaction (not yet reviewed by the user)
- **macOS three-column Moods** (agent, verified in the VM): sidebar | list ("Moods" + +) | detail (mood name at the
  toolbar's leading edge, Use This Mood / "Current mood", then Now's reload/stop; on another mood it switches, then
  paints: `AppModel.newWallpaper(from:)`). Esc goes to a focused text field first. Installed in /Applications.
  Seen in its screenshot: the keyword list's hint line is clipped at the bottom of the keyword box (to fix).
- **Website**: all features on the landing page + help/shortcuts/reference/privacy/network pages; PRIVACY.md and
  NETWORK.md at the repo root; Lighthouse 100s (A11y/BP/SEO) on all 8 pages, light/dark, desktop/mobile. One table on
  network.html scrolls inside its own box at 390 px (page doesn't). Preview: `python3 -m http.server 8737` in `_site/`.
- **Gemini painting 404** (user report "failed every recent session"): painting used `/v1`, which lacks preview models
  and Nano Banana 2.1; now `/v1beta` like the model list (live-verified with gemini-nano-banana-2.1).
- **Error redaction** no longer hides model names ("models/gemini-3-pro-image-preview").
- **Writer retries say what was wrong** (`ComposeContext.corrections`), and "none of the ideas followed the keywords"
  now names the most common problem (in the log). The user's failing mood's keywords are still unknown; the macOS
  wording for it is still the generic "The answer from your provider couldn't be used." → proposal: a typed error
  with a link to the mood (FFI change; batch with the Windows/Linux parity round).

## Release v0.1.0 (user, 2026-10-06: "ship it like we did AudioPaper")
Decided by the user: **all three platforms together**; **public repo github.com/msitarzewski/AutoPaper, one first commit
on main** (commits/push authorised for this release); **Windows: Azure Artifact Signing** ($9.99/mo; the user sets up
the account + identity check; signed MSIX on GitHub Releases + winget + App Installer update feed on the site);
**Linux: Flathub + a .flatpak on GitHub** (ask before opening the Flathub PR). Mac: as AudioPaper (Developer ID,
notarized DMG, Sparkle zip + appcast on Pages; signing env `~/.config/brew-browser/signing.env`). Bundle/app IDs stay
`com.autopaper` (Mac, like `com.audiopaper`) and `io.github.msitarzewski.AutoPaper` (Linux).
Core change made first: `AutoPaperError::KeywordNotFollowed { keyword, weight, mood_id }` (hosts: one line + link
to the mood). Then four agents: Windows (parity + lock screen restore + MSIX/App Installer/winget), Linux (parity +
Flatpak/Flathub), Mac (Sparkle + release script + new error), Repo (README/SECURITY/CONTRIBUTING, CI, gitleaks,
site links/downloads). Publishing steps (repo create, push, release, winget/Flathub PRs) are done by me, in order,
after review.

Agent results so far: **Repo** done (README/SECURITY/CONTRIBUTING, CI, gitleaks clean, site links, PRIVACY/NETWORK
update sections; repo settings list in its report: enable private vulnerability reporting + **Dependabot alerts on**
(user)). **Mac** done (Sparkle 2.10.0, release.sh/appcast.py, notarization of app + DMG accepted, VM Gatekeeper and
update tests, KeywordNotFollowed link); the Sparkle signing needs the user to click **Always Allow** for the
"AutoPaper" Keychain key at the first `scripts/release.sh` run. Windows and Linux agents still running.
**Windows** done (parity incl. Moods summary/chart, Grid|Gallery, provenance, reload/stop Ctrl+R/F5, Pause keeps,
lock screen keep/restore with an honest Spotlight note; manifest adds `picturesLibrary` + `unvirtualizedResources`;
`scripts/windows-release.ps1` packs a self-contained x64+arm64 .msixbundle (~391 MB), App Installer file, winget
1.12.0 manifests; verified install/upgrade/App Installer update in the VM with the dev cert). Waits on the user's
Artifact Signing account (env vars + one-time setup in the agent's report). **The committed
`site/static/AutoPaper.appinstaller` and `packaging/winget/manifests/**` are DEV outputs: regenerate with the real
signature or hold them out of the first commit.** To do: PRIVACY.md line for `picturesLibrary` (reads the lock screen
picture only); correct docs/research/windows.md (Spotlight → Picture needs registry virtualization off); test App
Installer against GitHub release URLs (redirect + octet-stream) right after the repo exists; Windows updates are
silent (land by the next launch). The 3 old `.env` copies in the Windows VM were deleted (user OK'd, 2026-10-06);
all robocopy syncs now use `/XF .env .env.*`.
Before the first commit: one-off `cargo fmt` (with a rustfmt.toml matching the code's width) then add the fmt check.
Store links (winget/Flathub) appear on the site only after those listings are accepted.

**Publishing the repo (user, 2026-10-06: "Build an AMAZING readme … then commit and publish! … github donate button …
MIT open source and we're taking PRs"; "center it with bedrock" → centered header like BEDROCK's mark + BEDROCK in
Other projects).** README rewritten (centered header, badges incl. PRs welcome + Sponsor, gallery, Built with Agency
Agents, Other projects incl. AudioPaper/Agency Agents/BEDROCK, big Sponsor button). Site switched to "Coming soon"
(release hero kept in `.scratch/held/index-release-hero.html`; restore at release). Dev-signed
`AutoPaper.appinstaller` and winget manifests moved to `.scratch/held/` (regenerated by the release script).
AGENTS.md (from ~/Clean) copied to the root. Waiting for the Linux agent's compiling checkpoint, then: final gitleaks,
branch → main, one commit, `gh repo create` (settings from the repo agent's list; Dependabot alerts on, private
vulnerability reporting on), push, Pages = GitHub Actions, watch CI + Pages.

**Published 2026-10-06:** https://github.com/msitarzewski/AutoPaper (public; first commit 437ae99 on `main`, 444
files, gitleaks clean). Settings: topics, Issues/Wiki/Projects on, Discussions off, Pages = GitHub Actions (live:
https://msitarzewski.github.io/AutoPaper/, "Coming soon"), Dependabot alerts on, private vulnerability reporting on,
secret scanning + push protection on, default workflow permissions read. Linux agent resumed after the commit (open:
Flatpak suspend/resume live check; own-wallpaper restore impossible in Flatpak without host dconf; **Flathub's
generative-AI policy: manifests must not be AI-written and a person must open the PR** — needs the user).
Release v0.1.0 still waits on: Windows Artifact Signing (user), the Sparkle Keychain "Always Allow" (user), restoring
the site's release hero + held Windows feed files, a v0.1.0 tag.

**Linux agent final (2026-10-06):** all 8 parity items done (AdwNavigationSplitView sidebar with moods under Moods,
mood page with GtkEditableLabel name, summary chart, Grid|Gallery, provenance, reload/stop, Pause keeps,
KeywordNotFollowed link, About/Help F1); 29 tests; Flatpak on GNOME 51 builds/installs/runs; `scripts/linux-release.sh`
makes the bundle (aarch64 only so far; x86_64 needs CI or an x86 machine). **Decisions for the user:** (1) Flathub's
generative-AI policy forbids AI-written manifests and AI-opened PRs ("Disclosure does not exempt manifests") — the user
would write the Flathub manifest and open the PR, or skip Flathub; (2) own-wallpaper restore in Flatpak needs host
dconf (linter errors) — alternative: the person picks their picture once via the FileChooser portal and Restore puts it
back through the Wallpaper portal. Doc fixes it listed (rust-linux.md manifest sketch, techContext finish-args and VM
clock, systemPatterns + app-spec GNOME moods layout) are still to do.
**CI:** first run: Windows/Linux app builds and core on Ubuntu/macOS passed; fixed in 8af425c: gitleaks pinned to
8.30.1, colour codes stripped from rustc's link list in build-xcframework.sh, Windows path separator in an engine test.

**Self-hosting progress (2026-10-06):** pipx setup ran (user): `/srv/www/msitarzewski.com/app-updates/autopaper/{flatpak,windows}`
live, Caddy lines in place (backup `.bak.1791342113`). CI fully green at b96c6c9. Docs agent: README/help/PRIVACY/
NETWORK/CONTRIBUTING/pages.yml switched to msitarzewski.com (uncommitted). Windows agent: `windows-release.ps1` writes
the feed beside the bundle (feed at the base, bundles under windows/, versioned), `scripts/publish-windows.sh` (bundle
first, feed last, HTTPS checks, keeps 3, refuses dev feeds on the real name); tested install + silent update from the
server on a test path, cleaned up. Release flow: VM `windows-release.ps1 -Version <v> -CopyTo \\Mac\Home\Clean\autopaper`
→ Mac `scripts/publish-windows.sh <v>` → attach the bundle to the GitHub release (winget URL). Waiting on the Linux
agent (own Flatpak repo + CI workflow + publish-flatpak.sh), then one commit.

## Next (in order)
1. The four release agents; review; the user's Artifact Signing account.
2. Publish: create the repo, first commit, push, Pages; build + sign artifacts; GitHub release v0.1.0; feeds last.
3. winget-pkgs and Flathub submissions (ask the user before each).
4. Backlog awaiting the user's go-ahead: **"On this Mac" writer** (Apple Foundation Models, text only) and **ChatGPT plan
   writer** (Sign in with ChatGPT, open-source preview: Responses API only, streaming, no temperature, no image
   generation — `docs/research/built-in-models.md` + conversation 2026-10-06); **Foundry Local** preset on Windows;
   manual "Paint with Image Playground…" (interactive sheet only; ImageCreator deprecated in macOS 27, verified in SDK).
5. Verify Nano Banana 2.1's price before making it a default; verify Gemini Omni's image output before offering it.

## Standing user decisions (2026-10-05/06)
- Native apps on macOS/Windows/Linux, native UX only, accessible (WCAG 2.2 AA via WCAG2ICT). Shared Rust core + UniFFI.
- Providers: OpenAI, Google Gemini, local/OpenAI-compatible (Ollama, LM Studio, ComfyUI); users' own keys.
- v1: wallpaper + lock screen + memory/echoes + moods; "Atmosphere" (accent, light/dark, sounds) is v2.
- MIT licence; Sparkle / MSIX+winget / Flatpak; site examples CC BY 4.0; the site credits "working with … Claude".
- Brand: AudioPaper's red + the display icon with Heroicons sparkles (not purple). The site is AudioPaper-styled.
- Errors: **one problem, said once, as a link to the fix** (app-spec rule 6a) — never directions.
- macOS shows wallpapers **over** the real wallpaper by default (AudioPaper's DesktopOverlay); "As my wallpaper" for
  the lock screen/Mission Control. **Pause keeps the current wallpaper**; Restore My Wallpaper / Quit uncover it.
- Local painting at **the largest size each model supports** (Apple-silicon VAE cap for 8× VAEs: 2,965,760 px);
  per-model bundled ComfyUI workflows (Z-Image Turbo default, Krea 2 Turbo, Qwen-Image 2.1) + "Your own" workflow.
- Keep **per-machine performance history** → progress, time left, learned timeouts, estimates, early starts.
- Moods = named keyword sets + their own Surprise; macOS: sidebar (Moods disclosure) › list (+, right-click) › detail
  (name in the title position); "Your Moods" dashboard when nothing is selected.
- "New Wallpaper Now" restarts the schedule; narrow keywords → stop retrying when it doesn't help + a note.
- Never GUI-test on the user's Mac (use the macOS test VM); never SendMessage agents a Workflow is running.
- Push notifications only when the user is needed or a build is ready.

## Environment facts
- Installed app: `/Applications/AutoPaper.app` (Release, Developer ID) via `scripts/macos-install.sh [--open]`.
- macOS test VM: `scripts/macvm/macvm.sh` (tart `autopaper-mac26`, headless; only `scripts/macvm` is shared — never the
  repo or `.env`). Windows VM "Windows 11", Linux VM "Scratch" (see techContext).
- ComfyUI on this Mac: `~/Software/ComfyUI`, `./run.sh`, 127.0.0.1:8188 (running).
- The user's real wallpaper is a macOS Aerial (no public API to restore it; the wallpaper store backup is in
  `.scratch/macos-wallpaper-backup/`).
- Website waits on the public repo: TODO comments for Download/Source/Issues links, `REPO = None` in
  `scripts/build_site.py`, assumed URL https://msitarzewski.github.io/AutoPaper/. Creating the repo/pushing needs the user.
- Round findings/outcomes: `.scratch/round3/` (r3-audit-*, r3-fix-*). Research: `docs/research/*.md`.
