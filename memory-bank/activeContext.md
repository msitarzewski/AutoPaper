# Active Context

**State (2026-10-07): v0.1.0 RELEASED** for macOS, Windows and Linux. Public repo https://github.com/msitarzewski/AutoPaper
(MIT, branch `main`, tag `v0.1.0`), site https://msitarzewski.github.io/AutoPaper/. CI green. Next work is optional
polish and the backlog (below); the user has a couple of UI tweaks in mind. Spec for every app: `docs/app-spec.md`;
design: `systemPatterns.md`; toolchains/VMs/infrastructure: `techContext.md`; history: `progress.md`,
`tasks/2026-10/`; decisions: `decisions.md`.

**Commit rule for this repo:** the user authorised commits/pushes for the release and its follow-ups (global rule otherwise:
commit only when asked). Commit trailer: `Co-Authored-By: Claude <model> <noreply@anthropic.com>`; never put Claude session
URLs anywhere.

## What shipped (all verified end to end)
- **macOS** — notarized DMG + Sparkle 2.10.0 zip on the GitHub release; feed `https://msitarzewski.github.io/AutoPaper/appcast.xml`
  (`site/static/appcast.xml`, pushed LAST at release time). Real-key update test passed in the VM (0.0.9 → 0.1.0).
  Bundle ID `com.autopaper`, team 7JQGQ7CRH8, installed on the user's Mac via `scripts/macos-install.sh`.
- **Windows** — signed MSIX bundle (x64+arm64, self-contained, ~391 MB) + `AutoPaper.appinstaller` served from
  `https://msitarzewski.com/app-updates/autopaper/` (bundle under `windows/`), also attached to the GitHub release.
  Signed with **Azure Artifact Signing** (account `msitarzewski-signing`, East US, endpoint `https://eus.codesigning.azure.net/`,
  profile `autopaper`, PublicTrust; identity validation completed in hours). Certificates are short-lived (3 days) and
  signatures are timestamped. Publisher/subject `CN=Michael Sitarzewski, O=Michael Sitarzewski, L=Dallas, S=tx, C=US`
  (lowercase `tx` comes from the user's billing address). Package family `AutoPaper_0a7ap1vxmmc6t`. Updates are silent
  (App Installer, by the next launch). winget manifests are in `packaging/winget/` — **not yet submitted** to
  microsoft/winget-pkgs (optional; the docs say "once reviewed").
- **Linux** — own GPG-signed Flatpak repository `https://msitarzewski.com/app-updates/autopaper/flatpak` (key fingerprint
  `6D6197DA7830EF13C5C25AC1005CC961E908702D`, public key `packaging/flatpak/autopaper-repo.gpg`), one-click
  `AutoPaper.flatpakref`, signed `.flatpak` files on the release. **No Flathub** (its AI-generated-manifest policy;
  user: "fuck Flathub"). Ubuntu needs `sudo apt install flatpak` first. Inside Flatpak, restoring the person's own
  wallpaper is impossible without host dconf (hidden, explained in Preferences); the FileChooser-portal "pick your picture
  once" alternative was offered and not chosen yet.
- **Website** — 8 pages (Home, Help, Shortcuts, Reference, Privacy, Network, Credits, 404), Lighthouse 100/100/100 light+dark,
  built by `scripts/build_site.py`, published by `.github/workflows/pages.yml`. Examples CC BY 4.0, five painted with
  Qwen-Image 2.1 (user: "NOT true [that it's research-only]. Leave them as is" — keep; link the Qwen team's clarification
  in Credits if the user provides it).
- **Repo** — README (centered header like BEDROCK, badges incl. PRs welcome + Sponsor, gallery, Built with Agency Agents,
  Other projects), SECURITY, CONTRIBUTING, THIRD-PARTY-NOTICES (generated crate list), AGENTS.md copy, FUNDING, issue
  templates. Settings: Issues/Wiki/Projects on, Discussions off, Pages = Actions, Dependabot alerts on, private vulnerability
  reporting on, secret scanning + push protection on, default workflow permissions read.

## Release recipes (next release; versions in `Cargo.toml` workspace, `apps/macos/project.yml`, Linux metainfo `<release>`)
1. **Mac:** `set -a; source ~/.config/brew-browser/signing.env; set +a; scripts/release.sh` → `build/release/AutoPaper-X.dmg|zip|sha256`
   and the feed entry in `site/static/appcast.xml` (leave it uncommitted until the GitHub release exists).
2. **Windows:** in the Windows VM `scripts\windows-release.ps1 -Version X -CopyTo \\Mac\Home\Clean\autopaper` with
   `AUTOPAPER_SIGN_ENDPOINT/ACCOUNT/PROFILE` and `AUTOPAPER_SIGN_PUBLISHER` set and `az login --use-device-code` done in the
   VM (user approves in a browser; Azure CLI 2.91 + x64 .NET 8 runtime are installed there); then on the Mac
   `scripts/publish-windows.sh X` (bundle first, feed last; keeps 3 bundles; refuses dev feeds).
3. **Linux:** tag `vX` and push (the `Flatpak` workflow builds x86_64 + aarch64), then `scripts/publish-flatpak.sh --tag vX`
   (Docker signs with `~/.config/autopaper/flatpak-gnupg`, rsyncs to pipx in four passes), upload `build/flatpak/release/*`.
4. `gh release create vX` with the DMG, zip, sha256s, flatpaks, msixbundle, appinstaller; THEN commit + push the appcast and
   site; then check links, the feed, and a Flatpak install in the Scratch VM. Update the home hero/README/Help if platforms change.

## Open items
- **Back up `~/.config/autopaper/flatpak-gnupg/`** (no passphrase; loss = no Linux updates for installed copies; the user
  confirmed this is done) and the Sparkle private key (login Keychain, account "AutoPaper"; loss = no Mac updates).
- winget submission (user decides); Linux own-wallpaper restore picker; extra empty Azure subscription "Michael - App
  Signing" (user: leave it); the user's UI tweaks (pending, unspecified).
- Backlog awaiting the user's go-ahead: **"On this Mac" writer** (Apple Foundation Models, text only), **ChatGPT-plan writer**
  (Sign in with ChatGPT: Responses API only, streaming, no temperature, no image generation — `docs/research/built-in-models.md`),
  **Foundry Local** preset on Windows, manual "Paint with Image Playground…" (ImageCreator is deprecated in macOS 27; verified).
- Known gaps/ideas: the user's failing mood's keywords (the "ideas didn't follow the keywords" report) were never identified, only
  the retry feedback and typed `KeywordNotFollowed` error were added; full VoiceOver/Narrator/Orca passes are still to do; no
  `cargo fmt` check in CI (≈1,440 diffs: needs a one-off format first); `docs/research/rust-linux.md`'s manifest sketch is stale;
  Nano Banana 2.1's price and Gemini Omni image output not yet verified before making them defaults.

## Standing user decisions (2026-10-05..07)
- Native apps on macOS/Windows/Linux, native UX only, accessible (WCAG 2.2 AA via WCAG2ICT). Shared Rust core + UniFFI.
- Providers: OpenAI, Google Gemini, local/OpenAI-compatible (Ollama, LM Studio, ComfyUI); users' own keys.
- v1 = wallpaper + lock screen + memory/echoes + moods; "Atmosphere" (accent, light/dark, sounds) is v2.
- MIT licence; Sparkle / App Installer (+winget) / own Flatpak repo; the site credits "working with … Claude"; Brand = AudioPaper's
  red + the display icon with Heroicons sparkles. The site is AudioPaper-styled, no app screenshots on the site.
- **Errors: one problem, said once, as a link to the fix** (app-spec rule 6a), never directions.
- macOS shows wallpapers **over** the real wallpaper by default (DesktopOverlay); "As my wallpaper" for lock screen/Mission
  Control. **Pause keeps the current wallpaper** on all platforms; Restore My Wallpaper / Quit uncover/restore the person's own
  (Windows also restores the lock screen picture; Linux only on GNOME outside Flatpak).
- Local painting at **the largest size each model supports** (Apple-silicon VAE cap for 8× VAEs: 2,965,760 px); per-model bundled
  ComfyUI workflows (Z-Image Turbo default, Krea 2 Turbo, Qwen-Image 2.1) + "Your own". Keep per-machine performance history.
- Moods = named keyword sets + own Surprise; macOS three-column Moods (sidebar › list › detail), reload/stop in the detail's toolbar.
- Never GUI-test on the user's Mac (use the macOS test VM); never SendMessage agents a Workflow is running; never copy `.env`
  anywhere (all robocopy syncs use `/XF .env .env.*`; three stray copies in the Windows VM were deleted 2026-10-06).
- Push notifications only when the user is needed or a build is ready.

## Environment facts
- Installed app: `/Applications/AutoPaper.app` (the 0.1.0 Release build, Developer ID, no get-task-allow).
- macOS test VM: `scripts/macvm/macvm.sh` (tart `autopaper-mac26`, throwaway `autopaper-mac26-run`; only `scripts/macvm` is shared).
  Windows VM "Windows 11" (ARM64), Linux VM "Scratch" (Ubuntu 26.04, GNOME 50). See `techContext.md`.
- ComfyUI: `~/Software/ComfyUI`, `./run.sh`, 127.0.0.1:8188. The user's real wallpaper is a macOS Aerial (no public API to restore).
- Hosting: pipx (`ssh pipx`) serves msitarzewski.com; docs `~/Clean/pipx/DEPLOYING.md`; setup script `scripts/pipx-app-updates.sh`.
- The user's Azure: signed in on the Mac with `az login` (and in the Windows VM); subscription "Azure subscription 1"
  (fc33e2dc-8746-4f5d-8733-5551655f8313), resource group `app-signing`; the user holds Identity Verifier + Certificate Profile Signer.
- Keys for live provider checks are in `.env` (git-ignored, mode 600): never print or share.
