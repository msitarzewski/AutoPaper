# AutoPaper app spec (macOS, Windows, Linux)

What every AutoPaper app does, so the three native apps match in behaviour while each looks and works like its own
platform. The design behind it is `memory-bank/systemPatterns.md`; the engine API is `core/src/engine.rs`.
Where this spec names a control, use the platform's own equivalent (HIG / Fluent / GNOME HIG) — never a custom
control where a stock one exists. Deviations from a platform guideline are called out to the user with the reason.

User preference (2026-10-07): use the platform's native list and form appearance throughout. List controls determine
row spacing, insets, typography, selection, separators and navigation icon sizes for their native context. Keep the
sidebar icons and use native icon/label controls. Form sections and preference/settings groups provide hierarchy
through their own headings and spacing; remove redundant app-drawn rules and header bands. Data, editable controls,
context actions, accessibility and functional image/list viewport geometry remain part of the app's content.

## Identity
| | macOS | Windows | Linux |
|---|---|---|---|
| App ID | `com.autopaper` (team 7JQGQ7CRH8, as AudioPaper) | MSIX `AutoPaper`, publisher from the dev cert | `io.github.msitarzewski.AutoPaper` |
| Data dir (`EngineConfig.data_dir`) | Application Support/AutoPaper (sandbox container) | `ApplicationData.Current.LocalFolder` (real `LocalState` path) | `$XDG_DATA_HOME/autopaper` (Flatpak's own) |
| Model dir | bundled `Models/bge-small-en-v1.5` | bundled in the package | installed data dir |
| Icon | `apps/macos/AppIcon.icon` (red display + sparkles) | same artwork as Fluent-style PNG assets | same artwork as a GNOME-style SVG |

Dev builds take the embedding model from the repo's `models/bge-small-en-v1.5` (`scripts/fetch-model.sh`). If it's
missing the engine falls back to reduced memory; `memory_status()` says so and Settings → Memory shows it.

## Voice
AudioPaper's: plain, specific, short. No hype, no "AI magic", no exclamation marks. macOS menu items and buttons in
title case (HIG); Windows and GNOME in sentence case. The same wording otherwise, so help and the website match.

## Engine lifecycle (all apps)
1. Launch: open `Engine` off the UI thread (`EngineConfig { data_dir, model_dir, locale, client: "<OS>/<version>
   AutoPaper/<version>" }`, the platform `SecretStore`). Call `set_display_hint(w, h)` with the largest display's pixel
   size, and again whenever displays change.
2. Timer: the platform's own scheduler (macOS `NSBackgroundActivityScheduler` or a coalesced timer; Windows
   `DispatcherQueueTimer`; Linux `glib::timeout_add_seconds_local`), checking `next_due()`; at the due time and on
   wake/resume/unlock call `run_if_due(observer)`. Every new wallpaper restarts the schedule (New Wallpaper Now,
   a disliked one's replacement and echoes too; revisits don't), so re-read `next_due()` after each one.
3a. **macOS: "Show wallpapers" — Over my wallpaper (default) · As my wallpaper** (user, 2026-10-06; AudioPaper's
   `DesktopOverlay`, `~/Software/AudioPaper/Packages/AudioPaperKit/Sources/AudioPaperKit/Wallpaper/DesktopOverlay.swift`
   + `FadeWindow` in `WallpaperService.swift:254`). *Over*: one borderless, click-through window per display at
   `CGWindowLevelForKey(.desktopWindow) + 1` (above the picture, below the icons), `[.canJoinAllSpaces, .stationary,
   .ignoresCycle, .fullScreenNone]`, cross-fading with `CATransition`; the person's real wallpaper (Aerials, dynamic) is
   never touched, so quitting or "Restore My Wallpaper" uncovers it exactly. **Pausing keeps AutoPaper's current
   wallpaper showing in both modes and only stops new ones** (user, 2026-10-06); the person's own wallpaper comes back
   only through Restore My Wallpaper (menus, Settings) or, Over my wallpaper, quitting. *As my wallpaper*: today's
   `setDesktopImageURL` path, needed for the lock screen and Mission Control; the lock-screen setting explains that it
   needs As my wallpaper. Switching modes releases the old one. Windows/Linux (no supported desktop-layer window): keep
   setting the real wallpaper, but record the person's own per display first and put it back when AutoPaper quits or the person chooses Restore My Wallpaper (Pause keeps the current
   AutoPaper wallpaper showing on every platform — user, 2026-10-06).
   Before composing, the core checks writing and painting independently in parallel with an 8-second limit per
   role, after the budget gate. Progress starts at CheckingServices. Console records both service_check outcomes
   and real GET requests; no prompts or paid calls are made if a check fails. New native actions return Shown:
   when services are unavailable, its ServicesUnavailable revisit points to the newest usable, non-disliked image
   in the active mood captured at run start (including explicit echoes). Missing/corrupt originals are skipped;
   if none remain, return the service error and leave the current wallpaper. Scheduled failures back off normally,
   with this fallback available on every failed check. A revisit must never trigger new-generation notifications.

3. Showing a wallpaper (new or revisited): for each display `render_for_display(id, DisplayTarget)` → set it as that
   display's wallpaper → set the lock screen when `settings.set_lock_screen` (where the OS allows) → `mark_shown(id)`.
   On display changes, re-render the current one for the new sizes.
4. Progress: show the stage from `ProgressObserver` (Checking services… · Composing an idea… · Checking memory… · Painting… · Downloading… ·
   Preparing for your displays…), with Cancel (`cancel()`: the call returns `Cancelled` promptly, even mid-request; a
   cancelled scheduled run fills its slot, so re-read `next_due()`). Observer calls arrive off the UI thread — marshal.
5. Ratings: `rate(id, rating)`; when it returns true, start `generate_or_revisit(Trigger::DislikeReplace, observer)`.
6. Errors: map each `AutoPaperError` variant to a plain sentence in the app's strings (e.g. MissingKey → "Add your
   OpenAI key in Settings to start.", RateLimited → "OpenAI asked AutoPaper to wait; it'll try again in N minutes.",
   Refused → "OpenAI declined to paint this idea; AutoPaper tried a gentler one."). A budget block explains the month's
   estimated spend, limit and next request cost, links to Budget settings, and keeps the current wallpaper.
   `InvalidInput` and `ProviderUnavailable` are worded by
   their typed `reason` (`InvalidInputReason`, `ProviderUnavailableReason`), which also says which field to point at;
   never show or match on `detail` (English, for logs). A `Shown.revisit` reason gets its own line.
6a. **One problem, said once, as a link to the fix** (user, 2026-10-06). A problem with a setting appears once per
   view — never both as a model-list error and a Test error — and is a link/button that goes straight to where it's
   fixed, not directions to follow: a missing key is **"Add your OpenAI key"**, which opens Settings → Keys/Accounts
   with that key's field focused; a refused key is **"Check your OpenAI key"**. Controls that can't work without the
   fix (Test, model Refresh) are disabled until it's done, and the view refreshes itself when it is (a saved key
   reloads the model list). Elsewhere (the Now view, a notification) the same link replaces "in Settings → …".
7. `keywords_are_narrow()` true → a gentle note on the Keywords view: "Your keywords are narrow, so new ideas are
   getting hard to find. Add some Maybes or raise Surprise for more variety."
8. Never block the UI thread on the engine: every engine call off the UI thread (blocking ones are documented).

## Surfaces

### 1. Always there (menu bar menu · notification-area menu · GNOME: background app + notifications; KDE tray)
Menu (top to bottom):
- The current wallpaper's title (disabled text), and its echo note on a second line when it's an echo.
- **Like** and **Dislike** (checkmarked for the current rating; choosing again clears it).
- **Mood ▸** a submenu of moods, the active one checkmarked, then **Edit Moods…** (opens Keywords).
- **New Wallpaper Now** (macOS ⌥⌘N from the menu bar, ⌘R from the Now and mood toolbars; Windows/Linux Ctrl+R and F5; the
  menu shows the shortcut where the platform does).
- While making one: the stage ("Painting…") and **Cancel**.
- **Pause New Wallpapers** / **Resume New Wallpapers**.
- **Show AutoPaper** (opens the main window).
- **Settings…** (macOS ⌘,) · **Quit AutoPaper**.
macOS: a menu, not a popover (HIG); template menu bar icon of the display + sparkles; the menu bar icon can be hidden
(then AutoPaper appears in the Dock, as AudioPaper). Dock icon only while the main window is open (AudioPaper's
`updateDockPresence`). Windows: WinUIEx `TrayIcon` with a native context menu; closing the window keeps the app in
the notification area. GNOME: no tray; the app runs in the background (Background portal) and the window is opened
from the app grid; KDE gets an SNI tray (ksni) with the same menu.

### 2. Main window — Now · Moods · History · Console (macOS: sidebar; Windows: NavigationView; GNOME: AdwNavigationSplitView / AdwViewSwitcher)
**Now**
- The current wallpaper large (from `thumb_path`, aspect-fit), its title, its summary (selectable text) and echo note.
  Accessible description = `describe(id)` (title, summary, echo note), plus the rating in the app's own words
  when it has one ("Liked"): `describe` leaves the rating to the host's language.
- Like / Dislike toggle buttons (state announced), **New Wallpaper Now**, and while generating the stage + Cancel.
- Status lines: next one ("Next new wallpaper at 9:00", or "Paused" / "Only when you ask"), budget ("$1.20 of $5.00
  this month (estimated)"), any error or revisit reason, the narrow-keywords note.
**Sidebar** (macOS, user 2026-10-06): Now · **Moods** (a disclosure group, expanded by default, listing every mood in the
person's order with the current one checkmarked — "Rainy beach, current mood" to VoiceOver; the same right-click menu as
the mood list; selecting a mood opens its detail, selecting the Moods row itself shows list › detail) · History · Console.
**Sidebar footer** (macOS, user 2026-10-06): a stationary footer at the bottom of the sidebar (doesn't scroll with the
list; `safeAreaInset(edge: .bottom)`), with a **Settings** gear button (`gearshape`, help "Settings (⌘,)", opens Settings
like the menu item) at the left and a quiet one-line status beside it (e.g. "Next wallpaper at 3:00 PM", "Painting…",
"Paused") — like the "Synced with iCloud" footer in first-party Mac apps. Windows/GNOME: Settings already live in the
NavigationView footer / the primary menu, so no change there.
**Moods with nothing selected** (user 2026-10-06): instead of "No Mood Selected", a summary of all moods — totals at the top
(moods, wallpapers made, liked), then one card per mood in the person's order: name, "Current mood" (checkmark + word) or a
Use button, its keywords grouped Must / Maybe / Avoid, Surprise band, wallpapers made, liked, last made (relative date), and
a small strip of its latest thumbnails; selecting a card selects that mood. Data from the engine (`mood_stats()`, additive),
not by paging history.
**Now toolbar** (macOS): a **New Wallpaper Now** toolbar button (`arrow.clockwise`, help "New Wallpaper Now", ⌘R) that
becomes **Stop** (`xmark`, cancels; Esc too) while one is being made, like Safari's reload/stop.
Windows/GNOME equivalents: Moods as an expandable NavigationView item with child moods / a sidebar section; a refresh
button in the Now page's command bar / header bar that turns into a stop button while working.

**Moods** (user's layout, 2026-10-06: sidebar › list › detail, like a first-party three-column Mac app)
- **List/navigation**: every mood, in the person's order (drag to reorder). macOS uses the middle column of a
  three-column `NavigationSplitView`; Windows and GNOME use their native navigation/list panes. macOS mood rows
  show names only (user, 2026-10-07); the active mood stays checkmarked in the sidebar. Use the platform's own row
  appearance. Double-click/Return uses a mood without generating. A **+** button in the list column's header/toolbar
  adds a mood (New Mood, named "New mood", name field focused in the detail). **Right-click** (context menu;
  long-press/Shift-F10 on other platforms) on a row: Use, Duplicate, Rename…, Delete… (Delete disabled for the last mood).
- **Detail pane** for the selected mood: its editable name. On macOS the name is in the detail header above its
  Surprise summary and statistics (user, 2026-10-07); the toolbar holds New Wallpaper Now / Stop at the trailing edge.
  Use is available from the context menu/Return; the repeated current labels and decorative mood icon are removed.
  Other platforms retain their own native mood activation controls. The form contains keywords (each row: text
  editable inline, Must / Maybe / Avoid choices, Remove; reorder by drag and keyboard Move Up/Down; accessible label
  "rain, Must"); "Add a keyword" (Return adds; default Must; a duplicate highlights the existing row); **Surprise**
  (slider 0–100, ends Faithful / Wild, band in words below; spoken "35 percent, fresh"); a short strip of recent
  wallpapers made in this mood (thumbnails, accessible `describe`); and **Delete Mood…** (destructive; native
  confirmation; disabled for the last mood). Group these using each platform's form/preference sections.
- "What AutoPaper has learned" (taste: global, not per mood) moves to Settings → Memory, with Reset….
- Keyboard: ⌘N in Moods = New Mood (the menu bar's New Wallpaper Now keeps ⌥⌘N or its own shortcut — no clash);
  Delete key on a selected row = Delete… (confirmation).
**History**
- Two views, switched in the toolbar and remembered (user, 2026-10-06): **Grid** and **Gallery** (like Finder's Gallery
  view: the selected wallpaper large, a filmstrip below, a details column — title, summary, echo note linked to its
  original, mood, keywords used, "Written by · Painted by · size · cost", date, rating, actions with Show on Desktop as
  the primary button; Return = Show on Desktop, Space = Quick Look/preview; arrow keys move along the filmstrip).
- Thumbnails anywhere else (a mood's "Made in this mood" strip, the moods summary): click = Show on Desktop; right-click
  = the same menu as History items (one shared menu builder).
- A grid of thumbnails, newest first, paged (`history(filter, limit, offset)`), filter: All · Liked · Disliked · Echoes,
  and a mood filter (All moods · each mood).
- Each item: thumbnail, title, date; badges for liked/disliked/echo (icon + text, not colour alone); accessible name =
  `describe(id)`, plus the rating in the app's words when rated.
- Actions (context menu + an actions button for keyboard users): **Show on Desktop** (revisit: render + set +
  `mark_shown`), **Like** / **Dislike**, **Make an Echo**, **Show Original and Echoes** (`lineage(id)`; offered when
  `has_echoes(id)`), **Show in
  Finder/Explorer/Files** (image path), **Delete…** (confirmation).

### 3. Settings (macOS Settings window with panes; Windows Settings page with SettingsCards; GNOME AdwPreferencesDialog)
**General**: New wallpaper (Every hour · 3 hours · 6 hours · 12 hours · **Every day** · Every week · Only when I ask);
Pause; Also set the lock screen (where supported; otherwise explain why it's unavailable); Replace wallpapers I dislike; Match my appearance (on by default: wallpapers suit light or dark mode);
When a new one can't be made (**Bring back one I liked** · Keep the current one); Open at login; macOS: Show in menu bar;
macOS: when Apple Intelligence can run, *On this Mac* is a writing choice (no key, address or model; its status and a link to System Settings when it isn't ready), and a first-run choice;
Windows/Linux: Notify me about new wallpapers (on by default there; macOS has no notifications by default).
**Providers**: two groups, **Writing ideas** and **Painting**: provider picker (OpenAI · Google Gemini · Ollama (writing
only) · OpenAI-compatible · ComfyUI (painting only) · Demo (no AI, gradients — for trying the app)); model picker filled
from `list_models` (with a Refresh), its blank choice named "Default (<`default_model(kind, job)`>)" when that isn't
empty; server address for local/compatible kinds (placeholder: `default_base_url(kind)`);
Painting quality (Standard · High); **Test** button with the result in words. ComfyUI: a **Workflow** choice —
**AutoPaper's** (default; the Model menu lists only installed models that have a bundled, tested workflow: Z-Image
Turbo (default), Krea 2 Turbo, Qwen-Image 2.1 — each with its own encoder, VAE and sampler) or **Your own…** (file
chooser; API-format JSON with `{{prompt}}`, `{{width}}`, `{{height}}`, `{{seed}}`; the Model menu becomes a read-only
line naming the model the file loads; "Choose another…"). A ComfyUI failure is one line + a link to Providers. Brew Browser help for local kinds per
systemPatterns ("Setting up local models"). Gemini note: "A free Gemini key may let Google use what you send to improve
its products; a paid project doesn't."
**Keys** (macOS "Accounts" like AudioPaper): OpenAI API key, Google Gemini API key, and the OpenAI-compatible server's key
(saved under `secret_account_for(selection)` — label "Key for <address>"). Secure fields that save as you type to the
platform store (no Save button), each with a "Get a key" link (platform.openai.com/api-keys, aistudio.google.com/apikey)
and its spoken label naming the service ("OpenAI API key").
**Memory**: What AutoPaper has learned (liked / disliked features from `taste_summary()`, **Reset…**); Quiet period (1 month · 3 months · **6 months** · 1 year · 2 years); Echoes (Off · Rarely · **Sometimes** ·
Often); memory status (`memory_status()`: full or reduced, and why); Storage: images on disk + size (`storage_usage()`),
Keep up to (256 MB · 512 MB · 1 GB · **2 GB** · 5 GB), **Clear History…** (confirmation offering "Keep memory" so AutoPaper
still avoids repeats; afterwards History and Now are empty — `current()` is none — while the desktop keeps its
picture), **Reset What It Learned…**.
**Budget**: Monthly budget (No limit · $1 · $2 · **$5** · $10 · $20 · custom), this month's estimated spend, estimated
cost per wallpaper and per month at the current cadence (`spend_summary()`), "Prices as of <`prices_as_of()`>".
**About**: icon, AutoPaper, version (build), one-line description, links (Website · Privacy · Help when they exist —
leave TODO comments until then), "MIT · © 2026 Michael Sitarzewski". Settings reopen on the last pane (AudioPaper).

### 4. First run
If no provider is configured beyond Demo: a welcome (macOS: a window/sheet; Windows: a first-run page; GNOME:
AdwStatusPage in the window) — "Tell your computer what you'd like to see." — three steps: add a few keywords (prefilled
suggestions are fine to *show* as examples, not to save), choose who writes and paints (OpenAI · Google Gemini · Local ·
Try it without AI), add a key if needed → **Make My First Wallpaper**. Skippable.

### 5. Notifications (Windows toast, Linux GNotification; macOS off unless the person turns it on)
"New wallpaper: <title>" with **Like** / **Dislike** actions (and **Show** opening the window). Never for routine
failures; one notification when the budget runs out and one when keys stop working.

### 6. Automation (macOS now; Windows/Linux later)
App Intents: New Wallpaper, Like This Wallpaper, Dislike This Wallpaper, What's on My Desktop (returns `describe`),
Switch Mood (a mood parameter; "Switch AutoPaper to Rainy beach").

## Accessibility (verify on each platform before saying done)
WCAG 2.2 AA via WCAG2ICT + each platform's guidelines. Every control and image has a spoken name; keyword rows speak
"rain, Must"; Surprise speaks value + band; rating buttons expose toggle state; history items speak `describe(id)`;
status changes are announced politely (progress stages, errors). Full keyboard operation (incl. reorder and history
actions). No meaning by colour alone; contrast ≥ 4.5:1 for custom text; respects Reduce Motion (no animated transitions
beyond the system's), Increase Contrast / high-contrast themes, Reduce Transparency. Audits: macOS AX API
(`AXUIElementCopyAttributeValue`, not System Events — AudioPaper's lesson), Windows UI Automation (`winapp ui inspect`,
AxeWindowsCLI), Linux AT-SPI (Accerciser/`pyatspi` dump) + an Orca pass where possible. Say what wasn't tested.

## Testing on the user's machines
- macOS runs on the user's own Mac: **record each screen's current wallpaper first and restore it after testing**
  (`NSWorkspace.desktopImageURL(for:)` / `setDesktopImageURL`), and capture only AutoPaper's own windows
  (`screencapture -x -o -l <windowID>`), never the whole screen (AudioPaper `projectRules.md`).
- Windows: the "Windows 11" VM; Linux: the "Scratch" VM (see `techContext.md` for access and screenshots). Their
  wallpapers may change freely, but restore at the end if easy.
- Use the **Demo** provider for UI work (free, offline); one end-to-end run with **ComfyUI** on this Mac
  (127.0.0.1:8188, running) proves real images; hosted providers wait for keys.
