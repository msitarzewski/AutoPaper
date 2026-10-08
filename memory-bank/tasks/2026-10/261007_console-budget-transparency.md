# 261007_console-budget-transparency

## Latest checkpoint — reboot handoff
This existing record now covers the entire local 0.1.1 follow-up batch: Console/budget transparency,
Console layout/help/statistics, Mac Moods cleanup, and native list/form refinement. The user explicitly
requested documentation of all tasks before reboot (2026-10-07 local / 2026-10-08 UTC).

Latest installed app: **macOS 0.1.1 build 5**. Latest QA: **459 core tests**, **107 Mac tests / 162 cases**,
**83 Windows tests**, **34 Linux tests**; 10 core and 1 Linux existing ignored tests. Builds and applicable
strict clippy pass. Source is uncommitted on `codex/console-runs`; no new public release. Details below
preserve the initial build 2 snapshot and record each subsequent completed task separately.

## Objective
Expose native date → runs → outcomes diagnostics for wallpaper generation, explain budget blocks exactly,
and keep the current wallpaper when a budget block prevents a new one.

## Initial outcome (build 2)
- Implementation approved; reviewed builds/tests passed on all platforms.
- macOS Release **0.1.1 build 2** installed and launched at the user's explicit request.
- Core: **393 unit + 62 integration passed**, 8 existing ignored; strict clippy clean.
- macOS: **106 tests / 161 parameterized cases**; Windows: **81 tests**; Linux: **31 tests**, 1 existing ignored.
- All native builds passed. VM budget blocks preserve current wallpaper and avoid key/network lookup.
- macOS/Linux native saved JSON equals the stored record; Clear preserves wallpaper memory/spend.
  macOS UTC month rollover resets the notice/spend without starting a run. Windows copy, budget link,
  unchanged wallpaper and Clear confirmation verified. Linux Budget link and actual clear verified.
- Windows native file saving remains unverified; Linux clipboard contents were not independently read.
  Live paid-provider GUI requests were not exercised; shared-core fixtures cover payloads/retries/errors.
- User authorized signing Windows 0.1.1. Both core architectures/signing prerequisites are ready, but a
  guest OS update interrupted packaging. Session recovered at 21:43:44 UTC. The user then explicitly
  deferred release work to batch more changes; the former budget-extension question is superseded.
  No fresh signed release artifacts, publication or commit for this update.

## Files Modified
- `core/src/model.rs:176` — run/event/status and budget records exposed through existing UniFFI.
- `core/src/store.rs:678` — append-only migration 5, run persistence, paging, retention, interruption recovery.
- `core/src/engine.rs:318` — Console API; run capture and sanitized HTTP trace; budget gate before credentials.
- `core/tests/engine.rs` and existing unit tests — budget/current preservation, truthful outcome/cost,
  trace/redaction/truncation, cancellation/interruption, retention/clear and sub-cent arithmetic regressions.
- `apps/macos/AutoPaper/Views/ConsoleView.swift:8`, existing AppModel/navigation/Scheduler/wording/tests —
  native Console, persistent budget notice, exact costs, UTC month refresh and original image prompts.
- `apps/windows/AutoPaper/Views/ConsolePage.xaml` and `.xaml.cs:363`, existing model/navigation/strings/tests —
  native Console and export/clear, budget notice/link, paging, original image prompts and exact costs.
- `apps/linux/src/console.rs:460`, existing app/window/Now/preferences/strings — native Console,
  persistent budget notice, UTC month/wake refresh, exact costs and original image prompts.
- `PRIVACY.md`, `NETWORK.md` — local trace retention/privacy and current-kept budget behavior.
- `apps/macos/project.yml` — user-assigned 0.1.1 / build 2; selected-file read/write for native JSON export.

## Reuse Analysis
Existing Engine/Store, HTTP dependencies, UniFFI, native app models/navigation, wordings and tests were extended.
History views require a generated image and cannot represent failed/blocked attempts with no wallpaper, so each
native platform needs its own Console view source. No new service, provider, database or external logging system.
For this task record, the October README is a summary and the existing v0.1.0 release task is historical;
separate task documentation preserves each approved update's outcome and rollback/QA evidence.

## Patterns Applied
- `memory-bank/systemPatterns.md#Storage (SQLite, PRAGMA user_version migrations)`
- `memory-bank/systemPatterns.md#Console run records`
- `memory-bank/systemPatterns.md#Scheduling and budget`
- `memory-bank/systemPatterns.md#Native surfaces (per platform HIG)`
- `memory-bank/systemPatterns.md#Testing seams`

## Integration Points
- `core/src/engine.rs:318` provides summary pages, full record/report and independent clear.
- `core/src/engine.rs:1228` wraps real run HTTP traffic, including model lookup/download errors.
- `core/src/engine.rs:1937` checks budget before credential readiness.
- `core/src/store.rs:678` persists bounded traces in the existing SQLite connection.
- Native hosts use their existing bridges/dispatchers/timers; Settings model tests do not become wallpaper runs.

## Architectural Decisions
Local bounded records provide request transparency without sending diagnostics elsewhere. Authentication headers,
known credential values and image bytes are omitted/redacted; truncation is explicit. Budget blocks always keep
the current picture, independently of optional provider-failure revisits. Requests from older builds are unavailable.

## Initial artifacts (superseded by later local builds)
- Branch: `codex/console-runs`; review diff: `/private/tmp/autopaper-console-review.patch`.
- Installed app: `/Applications/AutoPaper.app` (universal Developer ID signed Release 0.1.1 build 2).
- Strict installed signature verified; executable matches build SHA256
  `f839ce8e3fbf3330f4bf9517ce8befad8ac2c74f06278f7f583a33aac30dee8f`.
- Preview: `.scratch/shots/macvm/console-reserved-banner.png`; Linux: `.scratch/shots/linux/console-linux-final.png`;
  Windows: `.scratch/shots/windows/console-budget-blocked.png`.
- No host GUI tests. Installer only replaces app bundle; macOS privacy prevents direct tool assertions on the
  host container database. Original VM profiles/settings restored or untouched.
- Windows signed package: deferred at the user's request to batch releases. Never reuse stale cached versioned output.
- PR/commit/public release: not created for this update.

## Follow-up 1 — Console layout, help and statistics (local build 3)
**Requirement:** move retention/help text to website and History settings, use a full-height run column,
center empty messages, and show real success/failure/model timing charts as Console fills.

- Extended existing native Console views, Memory/History settings, and website help/reference sources.
- `core/src/engine.rs:342` exposes `console_statistics()`; `core/src/store.rs:722` aggregates retained runs.
  Migration 6 (`core/src/store.rs:258`) links new provider timings to exact run IDs; migration 7
  (`core/src/store.rs:264`) stores actual response models separately from requested estimate keys.
- Outcome/day counts cover retained runs. Success rate excludes blocks/cancellations/interruption;
  finished success/failure durations provide the average run time. Provider-call averages group actual
  model/provider/job and only exactly linked samples. Old timing samples are not assigned to old runs.
- Max 200 runs / 30 days and the existing 50 timings per provider/model/job limits remain independent.
- Mac/Linux charts, accessible tables, minimum widths and Clear were checked with actual run data.
  Windows real data/UIA passed; visual checks remain unavailable while locked.
- Core 459 tests, Mac 107/162, Windows 83, Linux 34 pass; native builds/strict clippy pass.
  Website built (8 pages); later Moods help edits passed 185 local link/ID/anchor checks.
- Local Mac build 3 installed at 22:22 UTC; superseded by builds 4 and 5. Public release deliberately deferred.

## Follow-up 2 — Mac Moods cleanup (local build 4; user approved)
**Requirement:** remove Use from the list, show names only, move the editable mood name into the detail
panel, remove the duplicate Current indicator and blue icon, and use right-click actions/delete confirmation.

- Extended `apps/macos/AutoPaper/Views/MoodsView.swift:140` and existing `MoodMenuItems`/`MoodDialogs`.
  Removed the obsolete MoodUseControl/MoodIcon; trailing toolbar keeps Make/Stop.
- Context Use changes the active mood without generating or changing the desktop; Delete retains native
  confirmation and last-mood protection. Duplicate/Rename and keyboard actions remain available.
- Fixed prematurely consumed initial name-focus requests; actual cross-row Rename/New Mood, Return/Esc,
  ordinary selection, Use, Delete/Cancel and 860px layout verified in the test VM.
- Mac 107 tests / 162 cases, universal Release and strict/deep Developer ID signature pass.
  Local build 4 installed at 22:50 UTC. User approved the result ("Nice") before requesting native defaults.
- Website help/shortcuts drafts updated; 8-page build and 185 local link/ID/anchor checks pass.
  Source integration captured in `docs/app-spec.md:119` and `systemPatterns.md#Native surfaces (per platform HIG)`.
- Evidence: `.scratch/shots/macvm/moods-build4-*.png`. Build 4 superseded by build 5.

## Follow-up 3 — Native list and form defaults (local build 5)
**Requirement:** each platform controls list spacing/fonts/selection/icon sizing; native form sections
replace distracting rules and header bands. Keep meaningful content, editing, context actions and accessibility.

- Mac: all five Lists audited (sidebar, Moods, keywords, Console, Original/Echoes). Native `.sidebar`/Label
  appearance retained. `apps/macos/AutoPaper/Views/MoodsView.swift:140` uses one grouped Form;
  `apps/macos/AutoPaper/Views/KeywordsView.swift:90` measures native content so 0/1 keywords fit naturally
  and longer sets scroll. Existing TitleTextField now retains requested focus until native attachment and
  successful first-responder acquisition. No custom icon tint forced after the VM used native monochrome.
- Windows: `apps/windows/AutoPaper/Views/MoodsPage.xaml:116` uses default list containers/RadioButtons;
  stock labeled mood-name TextBox replaces custom title-field skin. Removed duplicate pane/header rules
  and runtime border assignments. History Filmstrip and lineage rows inherit native container metrics.
- Linux: existing sidebar uses AdwButtonContent; keyword/welcome forms use native boxed-list-separate
  cards and stock EntryRow/ToggleGroup. Obsolete row/compact CSS and private EntryRow layout mutation removed;
  Surprise is an ordinary PreferencesGroup. `apps/linux/src/console.rs:205` bounds Statistics expansion;
  `apps/linux/src/console.rs:283` lets the stacked run pane resize natively after charts collapse.
- Native semantic separators, functional thumbnails/image aspects, grids/cards/charts and responsive
  panes preserved. No new production files or cosmetic-only unit tests; existing suites plus actual VM
  editing/selection/menu/minimum-width checks provided verification.
- Final QA: Mac 107/162, Windows 83, Linux 34 (+1 existing ignored); all builds pass, zero Mac/Windows
  warnings/errors, Linux strict clippy clean. Root diff check and redacted patch secret scan pass.
- Mac actual 0/1/8 keyword forms, add/remove/weights/reorder, Return/Esc, Rename/New Mood/ordinary focus,
  Delete/Cancel, real Demo lineage, actual HTTP500 + successful run, native divider drag and actual
  14-event copy/export JSON equality verified. Windows UIA covers normal/500px geometry and editing/
  Filmstrip/lineage. Linux actual 8-keyword input/reorder/weights/Remove/Undo, light/dark 360px forms,
  zero-budget run with no requests and narrow collapsed-Statistics resize visually verified.
- VM QA cleanup complete. Windows original data/preferences/desktop/Spotlight restored. Scratch original
  schema 4, 106 wallpapers, background and settings preserved; temporary services/profile/caffeinate removed.
  Mac VM cleanly shut down. Historical process IDs must be rediscovered after reboot.
- Completed 23:52 UTC within the 23:55 deadline. Native visual feedback remains pending; this documentation
  was explicitly requested before reboot. All public release work stays deferred.

## Current artifacts and verification limits
- Installed `/Applications/AutoPaper.app`: **0.1.1 build 5**, universal x86_64 + arm64, Developer ID team
  **7JQGQ7CRH8**, strict/deep signature verified. Installed executable equals the final installer-rebuilt
  artifact SHA256 `be7ccba16d242b60418a2a264d83ad2eb7498935f3b4535f53de7c29a264444e`.
- Existing installer `scripts/macos-install.sh Release --open` used without source changes. No host GUI
  tests or host app-data deletion. Original host DB contents are not directly asserted (macOS privacy).
- Persistent patch: `.scratch/autopaper-console-review.patch`; `/private/tmp` copy may disappear on reboot.
  Windows core/binding receipt: `.scratch/windows/console-statistics-core-receipt.json` (ARM64/x64 smoke pass).
- Evidence: `.scratch/shots/macvm/native-build5-*.png`, `.scratch/shots/linux/native-linux-*.png`,
  `.scratch/windows/native-lists-validation.log`, `native-lists-uia.log`, `native-keyword-geometry.json`,
  `.scratch/mac-install.log`, `.scratch/session-log.jsonl`.
- Unverified: locked Windows visuals/physical keyboard/drag and native exported-file save; Linux model-picker
  popup behind isolated keyring prompt and independent clipboard contents; full screen-reader passes.
  Direct AT-SPI row activation was unavailable; selected outcome/details and pane resize were visually checked.
- No paid-provider GUI requests, secret copying, commits, push, new release package/signing or publication.
  Azure signing is ready for the eventual batch; rebuild final-source packages rather than reuse stale output.

## Resume after reboot
Start with `memory-bank/activeContext.md#Reboot checkpoint`, October README and `git status --short --branch`.
Keep `codex/console-runs` and its uncommitted changes. Rediscover VM/process state and await the user's UI
feedback or next requested refinement. Publish/sign only when the substantial release batch is requested.


## Follow-up 4 — Service availability and selected-mood fallback (local build 6)

### Objective and behavior
The user reported attempts failing because ComfyUI was stopped. Check both selected writing/inference and
painting services before composing, log both results in Console, and show the latest valid saved wallpaper
from the selected mood when either is unavailable. Extend the existing Engine/provider/Store/native paths;
no new production files or schema migration for this follow-up.

Both roles check independently in parallel after the budget gate, with an eight-second timeout and cancellation.
Reuse read-only model lists; ComfyUI always contacts `/object_info`, even for a custom workflow without a loader.
Check the chosen local workflow after both availability checks and before any paid work. Console retains both
outcomes and real requests/errors. A failure can return the latest usable non-disliked image in the active mood
captured at run start, including when an echo source belongs elsewhere. Skip missing/corrupt originals using
existing image limits. If none remain, keep the current wallpaper and return the logged service error.

Strict `generate` / `make_echo` retain their error contracts; the new `generate_or_revisit` /
`make_echo_or_revisit` return `Shown`. Native manual/dislike/echo/scheduled paths preserve revisit wording and
suppress new-image notifications. The Console run stays Failed, spending stays zero, no new generation is stored.
Repeated scheduled checks preserve ordinary backoff and can use this fallback every time; recovery resets it.

### Implementation and verification
- Shared integration: `core/src/engine.rs:500`, `core/src/engine.rs:2021`, `core/src/engine.rs:2074`,
  `core/src/model.rs:522`, `core/src/providers/mod.rs:151`, `core/src/providers/comfyui.rs:540`.
  CLI generation/echo also return saved results accurately. Existing native AppModel/app and wording files extended.
- Final shared source hash: `efb9b2feea74a418775daebee951e4ce7cb7a5ac9fca9cd98085ce69895b20c3` (engine.rs).
- Core **465 tests** (396 unit + 68 integration + 1 calibration), **6 CLI**, **10 existing ignored**; strict clippy passes.
  Six new regressions cover one/both services down, newest same-mood image/cross-mood echo, disliked/missing/corrupt
  images, no same-mood fallback/current preserved, repeated scheduled failures/recovery, parallel timeout and cancellation.
  Loopback HTTP tests required sandbox escalation; the authorized full run then passed without waiver.
- Mac **108 tests / 163 cases**, signed universal Release build 6 pass. VM verified both-down manual fallback,
  non-disliked replacement without a loop, explicit echo with a source in another mood, and final-source repeat
  with writing available/ComfyUI down. Real GET checks, no compose/painting POST, no extra generation,
  honest Console Failed and native saved-wallpaper notice. No host GUI tests.
- Linux **35 tests / 1 existing Secret Service ignored**, native build and strict clippy pass. Real Scratch
  manual fallback applied latest active-mood image through the native portal, showed notice and both Console errors,
  zero cost/no new image. Separate native echo/dislike/scheduled UI actions not exercised; shared routing/core tests cover them.
  Original schema 4, **107-image current baseline**, settings/state/background hashes and running app preserved;
  isolated QA profile/services/transport scripts and keep-awake removed.
- Windows final native build passes with 0 warnings/errors; **86 unit/contract tests pass**, 0 failed/skipped.
  Forced final ARM64/x64 core rebuild and all 31 host source/artifact receipt hashes match. ARM64 real new-API
  and x64 real runtime smokes pass; loaded DLLs match the final receipt. Windows VM suspended again at 02:57:59 UTC. No logged-in user: GUI/UIA unavailable;
  tests ran in SYSTEM service session without touching the original user profile/desktop.
  Fresh final receipt `.scratch/windows/service-preflight-core-receipt.json`; stale x64 source-mtime cache was
  caught and fixed by forcing the final rebuild, never reported as a fresh artifact.
- Website/help/privacy/network and app-spec sources updated; 8-page build and 185 local links/anchors pass.
  Memory Bank links and final diff check pass; full review patch secret scan reports no leaks.

### Installation and artifacts
The unchanged `scripts/macos-install.sh Release --open` installed/opened **0.1.1 build 6** at 02:52 UTC.
Installed `/Applications/AutoPaper.app` equals the final built executable; SHA256
`2437884218b47dc2756a4f2395aeb8adfa2bfc0d2b5b71d8f9c1bc32840817fb`, universal x86_64/arm64,
deep/strict Developer ID team **7JQGQ7CRH8** signature valid, running PID 55635 observed. Earlier worker
pre-install hash is superseded by the installer-rebuilt artifact. Verification receipt:
`.scratch/service-mac-install-verification.json`; review patch `.scratch/autopaper-console-review.patch`.
VM screenshots: `.scratch/shots/macvm/preflight-build6-final-now.png`,
`preflight-build6-final-checks.png`; Linux `.scratch/shots/linux/service-linux-final-run.png`,
`service-linux-console-details.png`. Source remains uncommitted on `codex/console-runs`.

Public release remains **v0.1.0**. No commit, push, tag, public packaging/signing, feed update or website publication.
Implementation/QA completed within the 30-minute budget; user feedback on the installed development build is pending.
Mac VM shut down, Windows and Scratch suspended again, original data/state/background preserved.
Final state and receipts are tracked in activeContext. No host GUI tests or .env/credential copies.
