# Active Context

## Service availability preflight (2026-10-07 local / 2026-10-08 UTC) [COMPLETE; MAC INSTALLED]

- State: DOCS complete, implementation authorized directly by the user. Started 02:31:58 UTC; completed
  before 03:02 UTC deadline, within the 30-minute task budget.
- Both selected writing/inference and painting services are checked independently in parallel before composing,
  after the budget gate. Read-only model-list endpoints; ComfyUI always contacts `/object_info`. Eight-second
  timeout per role, prompt cancellation, no paid diagnostic work. Check local workflow after both checks.
  Console records both results and actual requests; returning a saved image leaves the attempt Failed.
- When a check fails, native manual/dislike/echo/scheduled paths show the newest usable, non-disliked saved image
  from the active mood captured at run start; skip missing/corrupt originals. Echo source mood does not change
  this scope; generic scheduled fallback preference does not override it. No saved image means preserve current
  and return the logged service error. Scheduled failures retain backoff; revisits suppress new-image notifications.
- Final engine source SHA256 `efb9b2feea74a418775daebee951e4ce7cb7a5ac9fca9cd98085ce69895b20c3`.
  Core **465 + CLI 6** tests pass (10 existing ignored), strict clippy clean. Final core/source frozen.
- Mac **108 tests / 163 cases**, universal Developer ID Release pass; VM both-down/one-down/manual/dislike/
  cross-mood echo checks pass, no extra generations or paid requests. Final-source repeat passed.
  Unchanged installer installed/opened **0.1.1 build 6** at 02:52 UTC; installed/final-built executable equality,
  universal architectures, strict/deep team **7JQGQ7CRH8** signature and running PID **55635** verified.
  Final installed executable SHA256 `2437884218b47dc2756a4f2395aeb8adfa2bfc0d2b5b71d8f9c1bc32840817fb`.
  Mac VM app quit, clean shutdown, all VM variants stopped, supervisor exited; no host GUI tests.
- Linux **35 passed / 1 existing Secret Service ignored**, build/strict clippy pass. Native final manual call
  applied latest same-mood saved image, notice/Console both failures, zero cost/no new image verified. Native
  echo/dislike/scheduled UI actions not separately exercised; shared routing/core regressions cover them.
  Original schema 4/current **107-image baseline**, settings/state/background hashes and running app unchanged;
  isolated QA profile/services/transport scripts/keep-awake removed. Scratch restored to its original suspended
  state at 02:59 UTC. Earlier 106 count is historical.
- Windows native build and ARM64 smoke passed. Forced final ARM64/x64 core rebuild from final source passed;
  preserved source mtimes had caused a stale x64 cache, caught and fixed by forcing the rebuild. Fresh matching
  dual receipt has all 31 host hashes verified; final native 86 tests pass (0 failed/skipped) and ARM64 smoke
  passes. Final x64 real runtime smoke also passed; loaded DLLs match the final receipt. Windows restored
  to its original suspended state at 02:57:59 UTC. Original profile/desktop/clipboard untouched.
  No logged-in user: GUI/UIA unavailable.
- Website 8-page build/185 local links and Memory Bank links pass; docs/primary task record updated. Review patch
  `.scratch/autopaper-console-review.patch` is persistent and refreshed/scanned after final docs; `/private/tmp` copy optional.
- Work stays uncommitted on `codex/console-runs`. Public release remains **v0.1.0**; no release/signing/push/feed/
  website publication. Full details in follow-up 4 of `tasks/2026-10/261007_console-budget-transparency.md`.

## Reboot checkpoint (2026-10-07 local; 2026-10-08 UTC)

- User explicitly requested updating the Memory Bank and all task records before reboot.
  Documentation is now authorized; native UI feedback and a public release remain separate open items.
- Work lives on **`codex/console-runs`** in `/Users/michael/Clean/autopaper`, with uncommitted changes.
  Public release remains **v0.1.0**; local Mac is now **0.1.1 build 6**, installed/opened and verified at 02:52 UTC.
  Build 5 / 23:44 UTC was the earlier reboot baseline, superseded by service-preflight follow-up 4.
  Nothing was committed, pushed, tagged, published or signed as a new public release during these updates.
- All completed tasks are recorded in `tasks/2026-10/261007_console-budget-transparency.md`: initial
  Console/budget work, Console layout/help/statistics, Mac Moods cleanup, native list/form refinement,
  and service preflight/selected-mood fallback.
  The v0.1.0 release task remains historical and now links to these follow-ups.
- Earlier reboot verification: core **459 passed** (396 unit + 62 integration + 1 calibration; 10 existing ignored),
  macOS **107 tests / 162 cases**, Windows **83**, Linux **34 / 1 existing ignored**. Native builds pass;
  core/Linux strict clippy, final diff check and redacted review-patch secret scan passed.
- Installed Mac `/Applications/AutoPaper.app`: universal arm64/x86_64, strict/deep Developer ID signature,
  team **7JQGQ7CRH8**; current build 6 installed executable matches final built executable SHA256
  `2437884218b47dc2756a4f2395aeb8adfa2bfc0d2b5b71d8f9c1bc32840817fb`.
  Previously recorded PIDs are historical and must be rediscovered after reboot.
- QA cleanup completed: Mac test VM stopped, localhost fixture/supervisor stopped; Windows original
  LocalState/preferences/desktop/Spotlight restored, test app stopped; Scratch temporary QA services/profile/
  marker/snapshot/caffeinate removed, original app settings/data/background preserved (106 generations).
- Outstanding verification: Windows visual and physical keyboard/drag checks (locked VM), Windows native
  exported-file save, Linux model picker popup (isolated keyring prompt), independent Linux clipboard read,
  full VoiceOver/Narrator/Orca passes. Direct AT-SPI run-row activation was unavailable; selected Linux
  outcome/details and native pane resize were visually verified. No credentials were retrieved or entered.
- Persistent review artifact: **`.scratch/autopaper-console-review.patch`** (git-ignored, workspace disk).
  `/private/tmp/autopaper-console-review.patch` is only a convenience copy; do not rely on it after reboot.
  Current tracked/untracked source plus task records are the authoritative state. The patch can be regenerated
  from `git diff --binary` plus `git diff --no-index /dev/null <untracked-file>` for each untracked source/doc.
- Resume: read this checkpoint and October README, confirm branch/status, rediscover VM/process state,
  then continue from the implemented/verified build. Do not rerun completed work without a new change or
  failure. GUI-test only in the VMs; never copy `.env`. Keep the latest wallpaper on a budget block.
- Next: collect feedback on installed build 6; service preflight follow-up 4 supersedes build 5. Finish any requested refinements. Release packaging,
  Windows signing, notarization/feeds/website publication wait for an explicitly requested substantial
  batch. Rebuild final-source artifacts then; never reuse stale cached 0.1.1 packages.

## Native list and form defaults (2026-10-07) [MAC INSTALLED; REVIEW PENDING]

- State: local BUILD/QA/APPLY complete at 23:52 UTC; started 23:24 UTC, deadline 23:55 UTC.
  Implementation feedback pending; task documentation updated at the user's reboot request. Branch `codex/console-runs`.
- User requested each platform's native lists everywhere, retaining left-panel icons with native sizes.
  Added native form grouping after the distracting Add/Keywords rules screenshot. The sidebar reference
  guides native spacing, quiet headings and selection; each platform keeps its own native appearance.
- Reused existing app files, no new production files/tests for styling. Native list/control spacing,
  typography, selection and icon sizing replace app overrides; native form/preference sections replace
  redundant borders/header bands. Functional image sizes, split panes, cards, charts and actions remain.
- Mac: five native list surfaces audited and checked (sidebar, Moods, keywords, Console, lineage).
  `MoodsView.swift:140` now holds one grouped Form; `KeywordsView.swift:90` measures native list content
  so empty/short keyword sets fit naturally and longer lists scroll. Sidebar uses stock `.sidebar`/Label
  behavior; native tint attempts remained monochrome on VM macOS 26, so ineffective overrides removed.
  Existing TitleTextField retains requested focus until window attachment and successful first responder.
- Mac final Release build and 107 tests/162 cases pass, zero failures/warnings. VM verified 0/1/8 keywords,
  normal/minimum widths, add/remove/weights/reorder, Return/Esc, ordinary selection unfocused, cross-row
  Rename/New Mood focus, native menus/Delete/Cancel, real Demo original/echo, actual success/HTTP500
  Console rows, keyboard/divider resizing and real 14-event JSON copy/export equality. VM app/localhost
  fixture/supervisor stopped and VM shut down cleanly. Root reviewed final form, Console and lineage images.
- Authorized unchanged `scripts/macos-install.sh Release --open` installed/opened 0.1.1 build 5 at 23:44 UTC.
  Universal x86_64 + arm64, strict/deep Developer ID team 7JQGQ7CRH8 signature valid. Installed executable
  equals final installer-rebuilt artifact SHA256 `be7ccba16d242b60418a2a264d83ad2eb7498935f3b4535f53de7c29a264444e`;
  running PID 9716 verified. Earlier worker artifact hash superseded. No host GUI tests/data deletion.
- Windows: final Debug build (0 warnings/errors), 83 tests, ARM64 CoreSmoke, XAML parse/diff checks pass.
  UIA verified normal/500px native controls, rename/add/remove/weights, Filmstrip and lineage. Original
  VM data, checksummed preferences, desktop/Spotlight restored; QA app stopped. Visual and physical
  keyboard/drag checks remain unverified while locked; no credentials retrieved or entered.
- Linux: final build/strict clippy and 34 tests pass (1 existing Secret Service ignored). Native sidebar/
  model factory, separate keyword cards and unboxed Surprise group audited. Actual 8-keyword input,
  reorder/menu/remove/Undo/weights, light/dark and 360px layouts pass. Real zero-budget Console attempt
  sent no requests; native Statistics expansion allocation and stacked pane resizing corrected/verified.
  Model-popup runtime check remains unavailable behind isolated keyring prompt; source audited. Direct
  AT-SPI run-row activation unavailable, but native selected outcome/details and resize visually verified.
- Scratch unexpectedly suspended twice; resumed only Scratch and used temporary caffeinate, no power
  preference changes. QA profile/services/markers/snapshot/caffeinate removed. Original process 1942390,
  schema 4, 106 generations, background URI, pause=false/budget=500/Demo writer unchanged.
- Compaction recovery 23:39 UTC resumed existing QA. Existing app spec/system patterns record the user's
  native list/form preference and approved earlier Mac Moods cleanup; stale budget fallback wording fixed.
- Full review patch saved at `.scratch/autopaper-console-review.patch`; `/private/tmp` copy is disposable. No commits, pushes,
  public release packages/signing or website publication; user requested one substantial release batch.

## Mac mood UI cleanup (2026-10-07) [MAC INSTALLED; APPROVED]

- State: local BUILD/QA/APPLY complete by 2026-10-07T22:50:20.211887+00:00; started 22:32 UTC, within the 30-minute task budget.
  Installed/opened 0.1.1 build 4 as requested. User approved the result ("Nice") at 23:24 UTC and
  requested native list defaults as the next refinement. Public release stays deferred.
- Dedicated mood list shows names only. Editable `MoodTitleField` moved into the detail header above
  Surprise/stats. Removed row Use controls, duplicate detail/toolbar current status, toolbar name, and
  decorative blue icon. Existing flexible spacer keeps Make/Stop trailing. Sidebar active check remains.
- Reused `MoodMenuItems` (Use/Duplicate/Rename/Delete) and `MoodDialogs` native Delete confirmation.
  Fixed one prematurely handled initial name-focus request in `TitleTextField.makeNSView`, so Rename
  on a different row focuses the newly selected title. Removed unused MoodUseControl/MoodIcon.
- Existing Mac tests: 107 tests / 162 cases pass; Release universal build and deep/strict signature pass.
  VM verified actual right-click menu; Use changes active mood without a new run/generation or changed
  desktop URLs; Delete confirmation/Cancel and temporary QA mood deletion; Rename on selected/different
  rows, Return commit, Esc restore, New Mood focus, ordinary selection without activation or name focus.
  Normal and 860px layouts checked. Existing compact stat tiles ellipsize long labels/time at minimum
  width but remain AX-readable. Temporary QA mood deletion verified; app quit and VM shut down cleanly.
- Root reviewed final/normal, minimum-width, context-menu and delete screenshots in
  `.scratch/shots/macvm/moods-build4-*.png`. No host GUI testing; only authorized install/open and
  read-only signature/version/architecture/process/hash verification. No host app-data deletion.
- Unchanged `scripts/macos-install.sh Release --open` installed `/Applications/AutoPaper.app`.
  Final installed executable matches the installer-rebuilt artifact, universal x86_64 + arm64,
  Developer ID team 7JQGQ7CRH8, version 0.1.1 build 4, SHA256 `43d3b36607928b04982346f9d25ba490b378bd37057922941ca54594495ac521`. Running PID verified.
  Pre-installer worker artifact hashes are superseded by this final installed/build equality.
- Website help/shortcuts drafts describe the clean Mac list and relocated editable name. Eight pages
  built; 185 local links/unique IDs/anchors pass. Website publication deferred with the release batch.
- Branch `codex/console-runs`; no new source files/tests for this cleanup, commits, pushes, public
  release packages. The existing follow-up task record now includes this approved cleanup. Following user approval, the existing `docs/app-spec.md` and
  `systemPatterns.md` now describe the Mac list/title cleanup and native list/form preference.
- Review patch saved at `.scratch/autopaper-console-review.patch`; `/private/tmp` copy is disposable.
- Compaction recovery 22:35 UTC resumed this task without restarting prior Console work.

## Console and budget transparency (2026-10-07)

- State: refinement code/QA complete by 22:22:39 UTC (began 21:54 UTC, within 30-minute budget).
  macOS installed; all native builds/tests pass. Public release deferred by the user:
  gather more changes and publish/sign one batch, rather than a release for every build.
- Refinement: move retention/privacy help to website and History settings; full-height run column and
  centered empty states; real retained-run outcome/day charts and average provider-call duration by
  provider/job/model. macOS, Windows and Linux workers own their native app directories; root owns
  shared core and website. Migration 6 links new timing samples to exact run IDs; migration 7 stores
  actual response model names separately from the existing estimate keys. Older samples remain
  unlinked; no historical timing is inferred. All retained runs contribute to outcome/day statistics.
- Shared `console_statistics()` API/tests and strict clippy pass. Website help/reference updated;
  8-page build and 77 local links/unique IDs pass. Website remains unpublished for the batched release.
- macOS VM verifies real success/failure/block charts, 50% success with blocks excluded, accessible
  tables, full-height columns and centered empty states, including minimum width/divider resize.
  Linux normal/minimum-width charts and clear verified. Windows data/UIA checks pass, but the locked
  VM prevents visual screenshots; user was asked to unlock it. No credentials entered or retrieved.
- Windows final minimum-width empty correction verified via UIA: centered within 0.5 px normally,
  0 px at 500 × 520; labels fully within the window. Clear preserves both QA wallpapers/current ID;
  original LocalState, checksummed app preferences, desktop image and Spotlight state restored.
  Linux original data/wallpaper untouched, temporary profile removed; macOS VM shut down cleanly.
- Compaction recovery 21:59 UTC: resume existing work, not a fresh implementation. No commits/pushes.
- Branch `codex/console-runs`. Implementation authorized; after review and QA the user assigned 0.1.1
  and explicitly requested installing the new macOS app. Commits/publication were not requested.
- Native date → runs → outcomes Console exposes actual prompts, providers/models, responses, retries,
  errors, timing and estimated cost. Local bounded records omit credentials/image bytes; old requests
  cannot be reconstructed. Budget notices explain spent/limit/next estimate and keep the current wallpaper.
- Core: 396 unit + 62 integration + 1 calibration tests passed (10 existing ignored), clippy clean.
  macOS: 107 tests / 162 cases passed; Windows: 83 tests passed; Linux: 34 passed / 1 existing ignored.
  All native builds passed. VM budget/current-wallpaper checks passed on all three platforms.
- macOS/Linux native saved JSON matches the stored record, and Clear preserves wallpapers/spend.
  Windows copy and Clear confirmation verified; actual native exported file save remains unverified.
  Linux Copy callback exercised; clipboard contents were not independently verified.
- Earlier Console refinement installed and launched universal Developer ID signed macOS Release 0.1.1 build 3 at
  `/Applications/AutoPaper.app`; strict codesign passed, installed executable equals build SHA256
  `0b7b7869605e0ebd52f5eb2a0c53f22a07d9049e6c4a7ca27a207bd3f0b0e823`.
  Installer only replaced the app bundle; no host GUI testing or data deletion. macOS privacy blocks
  tool access to container DB contents, so no direct host DB assertions were made.
- Windows x64/ARM64 final core and bindings are built; Azure public signing profile/auth/runtime ready.
  OS update/restart interrupted fresh x64 packaging. Guest session recovered at 21:43:44 UTC after
  update to Windows build 10.0.29683.1000. No fresh signed 0.1.1 artifacts copied. User explicitly
  deferred release packaging/signing/publication at 22:17 UTC; the old signing budget-extension question
  is superseded. Final chart-capable ARM64/x64 DLLs/bindings match current source; both smoke tests pass.
  Receipt: `.scratch/windows/console-statistics-core-receipt.json`. When a batch release is requested,
  rebuild from its final source and verify signatures/timestamps/versions/DLL hashes; never use stale output.
- Persistent review patch: `.scratch/autopaper-console-review.patch`; regenerate after source/documentation changes.
  Task record: `tasks/2026-10/261007_console-budget-transparency.md`.

**State (2026-10-07): v0.1.0 RELEASED** for macOS, Windows and Linux. Public repo https://github.com/msitarzewski/AutoPaper
(MIT, branch `main`, tag `v0.1.0`), site https://msitarzewski.github.io/AutoPaper/. CI green. Next work is optional
polish and the backlog (below); the implemented Console/Moods/native form follow-ups are documented above. Spec for every app: `docs/app-spec.md`;
design: `systemPatterns.md`; toolchains/VMs/infrastructure: `techContext.md`; history: `progress.md`,
`tasks/2026-10/`; decisions: `decisions.md`.

**Commit rule for this repo:** historical authorization covered the v0.1.0 release. The current batch remains
uncommitted and unpublished; do not infer renewed commit/push/release approval from the reboot documentation request. Commit trailer: `Co-Authored-By: Claude <model> <noreply@anthropic.com>`; never put Claude session
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
  Signing" (user: leave it); user feedback on the installed native list/form build.
- Backlog awaiting the user's go-ahead: **"On this Mac" writer** (Apple Foundation Models, text only), **ChatGPT-plan writer**
  (Sign in with ChatGPT: Responses API only, streaming, no temperature, no image generation — `docs/research/built-in-models.md`),
  **Foundry Local** preset on Windows, manual "Paint with Image Playground…" (ImageCreator is deprecated in macOS 27; verified).
- Known gaps/ideas: the user's failing mood's keywords (the "ideas didn't follow the keywords" report) were never identified, only
  the retry feedback and typed `KeywordNotFollowed` error were added; full VoiceOver/Narrator/Orca passes are still to do; no
  `cargo fmt` check in CI (≈1,440 diffs: needs a one-off format first); `docs/research/rust-linux.md`'s manifest sketch is stale;
  Nano Banana 2.1's price and Gemini Omni image output not yet verified before making them defaults.

## Standing user decisions (2026-10-05..07)
- Batch changes into substantial releases; local development builds/installations do not need a public release each time.
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
- Installed app: `/Applications/AutoPaper.app` (0.1.1 build 6 Release, Developer ID, no get-task-allow).
- macOS test VM: `scripts/macvm/macvm.sh` (tart `autopaper-mac26`, throwaway `autopaper-mac26-run`; only `scripts/macvm` is shared).
  Windows VM "Windows 11" (ARM64), Linux VM "Scratch" (Ubuntu 26.04, GNOME 50). See `techContext.md`.
- ComfyUI: `~/Software/ComfyUI`, `./run.sh`, 127.0.0.1:8188. The user's real wallpaper is a macOS Aerial (no public API to restore).
- Hosting: pipx (`ssh pipx`) serves msitarzewski.com; docs `~/Clean/pipx/DEPLOYING.md`; setup script `scripts/pipx-app-updates.sh`.
- The user's Azure: signed in on the Mac with `az login` (and in the Windows VM); subscription "Azure subscription 1"
  (fc33e2dc-8746-4f5d-8733-5551655f8313), resource group `app-signing`; the user holds Identity Verifier + Certificate Profile Signer.
- Keys for live provider checks are in `.env` (git-ignored, mode 600): never print or share.
