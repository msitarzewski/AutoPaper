# October 2026

### 2026-10-07: Service availability preflight [COMPLETE; MAC INSTALLED]
- User authorized checking both selected services before wallpaper generation, logging both outcomes in Console,
  and showing the latest usable saved wallpaper from the selected mood when either service is unavailable.
- Started 02:31:58 UTC / deadline 03:02 UTC; implementation/QA complete on codex/console-runs.
  Shared core 465 + CLI 6 pass (10 existing ignored), strict clippy; Mac 108 tests / 163 cases,
  Linux 35 / 1 ignored and native builds pass.
  Mac/Linux VM service/fallback checks complete; Windows final dual DLL/native build/86 tests and ARM64 smoke pass,
  x64 runtime smoke passes and Windows VM suspended again. GUI unavailable without a logged-in user.
  Signed universal build 6 installed/opened and installed/build equality verified; public releases remain deferred.
- Details: [follow-up 4](261007_console-budget-transparency.md#follow-up-4--service-availability-and-selected-mood-fallback-local-build-6).

## Reboot checkpoint — 2026-10-07 local / 2026-10-08 UTC
All completed tasks below are now documented in the existing task records at the user's explicit request.
Local Mac: **0.1.1 build 6** (supersedes reboot baseline build 5); public release: **v0.1.0**; branch: **`codex/console-runs`**, uncommitted.
Read `../../activeContext.md#Reboot checkpoint` first after restart. Persistent review patch:
`.scratch/autopaper-console-review.patch`; do not rely on the `/private/tmp` copy.

### 2026-10-07: Native list/form defaults [MAC INSTALLED; REVIEW PENDING]
- Native row/icon/control metrics across all apps; redundant form rules replaced by native sections/groups.
  Mac keyword viewport fits short lists naturally; long lists scroll. Attachment-aware title focus preserves
  Rename/New Mood behavior. Linux collapsed Statistics and stacked run panes allocate available space correctly.
- Mac 107 tests / 162 cases, Windows 83 tests, Linux 34 tests (1 existing ignored), all native builds pass.
  VM form/list/menu/editing/narrow checks pass; Windows visuals/physical input unavailable while locked,
  Linux model-popup runtime unavailable behind isolated keyring prompt. See activeContext and [follow-up 3](261007_console-budget-transparency.md#follow-up-3--native-list-and-form-defaults-local-build-5) for exact scope.
- Mac signed universal 0.1.1 build 5 installed/opened and installed/build equality verified. VM QA cleanup
  complete; Windows/Scratch original data/preferences/desktop preserved or restored. No host GUI tests.
- Completed at 23:52 UTC, within deadline 23:55 UTC. Native UI feedback pending; all tasks documented at the reboot request;
  public releases/commits/pushes remain deferred for a larger batch.

### 2026-10-07: Mac mood UI cleanup [MAC INSTALLED; APPROVED]
- Names-only mood list; editable name in the detail panel; repeated current indicators and blue icon
  removed. Right-click Use/Delete retains native delete confirmation and other mood actions.
- Fixed initial focus for cross-row Rename; ordinary selection does not activate a mood or focus its name.
- Mac 107 tests / 162 cases passed, signed universal build verified. VM checked Use without new runs or
  wallpaper changes, Delete/Cancel, selected/cross-row Rename, New Mood, Return/Esc and minimum width.
- Release 0.1.1 build 4 installed/opened by request, strict Developer ID signature and final build hash
  equality verified. VM shut down cleanly; no host GUI tests. Within 30-minute task budget.
- Website help/shortcuts drafts updated; 8-page build and 185 local links/IDs/anchors pass.
- User approved the cleanup ("Nice"); next refinement is native list defaults across apps. No public
  release/commit/push; larger batch deferred.
- Task record: [follow-up 2](261007_console-budget-transparency.md#follow-up-2--mac-moods-cleanup-local-build-4-user-approved).

### 2026-10-07: Console and budget transparency [MAC INSTALLED; BATCH RELEASE DEFERRED]
- Full-height Console, centered empty states, help in History settings/website sources, and real
  outcome/day/model-duration charts implemented. Shared tests/clippy and macOS/Linux native QA pass;
  Windows normal/minimum-width UIA checks pass, visuals unavailable while locked. All QA complete
  at 22:22:39 UTC, within the 30-minute refinement budget; original VM states preserved/restored.
- Native date/run/outcome Console with actual prompts, models, responses, retries, costs and errors.
- Exact prospective budget notice; budget blocks keep the current wallpaper before any key/network lookup.
- Core 459 tests, macOS 107 tests / 162 cases, Windows 83 tests, Linux 34 tests passed; builds/clippy clean.
- macOS Release 0.1.1 build 3 installed/opened, Developer ID verified; superseded by build 5.
- User requested batching changes before public releases; packaging/signing/website publication deferred.
- Native exported JSON verified on macOS/Linux; Windows picker opens, actual file save unverified.
- See [261007_console-budget-transparency.md](261007_console-budget-transparency.md).

### 2026-10-05 → 2026-10-07: AutoPaper v1 build and v0.1.0 release [COMPLETED]
- Plan approved 2026-10-05; shared Rust core + native macOS/Windows/Linux apps built, audited (accessibility, security, code review)
  and fixed in rounds; moods, performance history, overlay mode, errors as links, three-column Moods, parity on all platforms.
- 2026-10-06: repo made public and CI fixed; Gemini painting `/v1beta` fix and `KeywordNotFollowed`; self-hosted updates on pipx
  (own Flatpak repo, App Installer feed) instead of Flathub.
- 2026-10-07: Azure Artifact Signing set up; **v0.1.0 released** for macOS, Windows and Linux; real-key Sparkle update test passed.
- Full account in [261007_autopaper-v0.1.0.md](261007_autopaper-v0.1.0.md); decisions in `../../decisions.md`; recipes for the next
  release in `../../activeContext.md`.
