# System Patterns

Status: **core built (2026-10-05)** — `autopaper-core` (every module, no `todo!()`), the `autopaper` CLI and the
macOS XCFramework/Swift bindings. All native apps shipped v0.1.0; local 0.1.1 Console/Moods/native form
follow-ups are built/verified, with Mac build 5 installed and public release deferred. Phase 2 core changes (2026-10-05, user
decisions): narrow keywords (retries stop when they don't help; `keywords_are_narrow`), every new wallpaper
restarts the schedule, old failed rows pruned, hosted timeouts as `ProviderUnavailable`, `describe` without
the rating, and the build scripts (`scripts/build-xcframework.sh`, `scripts/build-core-windows.ps1`). Core
follow-up after the three apps (2026-10-06, from the app builders' requests): typed error reasons
(`InvalidInputReason`, `ProviderUnavailableReason`), UniFFI exports (`prices_as_of`, `default_base_url`,
`default_model`, `surprise_band`, `Engine::has_echoes`), `cancel` interrupting a provider request under way (and
stopping a ComfyUI job), a cancelled scheduled run filling its slot, the desktop's renders kept through
clear/delete/prune, ComfyUI sizes from a per-model limits table, and Demo grammar. **Core round 4 (2026-10-06,
built): Moods, performance history and progress (timings, estimates, ComfyUI step progress over /ws, learned
timeouts, early scheduled starts), the typed painting failure (`PaintingFailed`) and Demo's slow mode** — see the
two sections near the end ("as built" notes there). Numbers below are the implemented ones; where the implementation
changed the design, the change is stated here (sources: the module doc comments).

## Shape
```
core/            autopaper-core — the agent: composer, memory, novelty, echo, taste, providers, scheduler,
                 image pipeline, storage. Rust library; UniFFI exports for Swift and C#; used directly by Linux.
cli/             autopaper — developer CLI over the core (like AudioPaper's apctl). Reads keys from .env.
apps/macos/      SwiftUI + AppKit app (XcodeGen), links the core as an XCFramework.
apps/windows/    WinUI 3 / C# app (Windows App SDK, MSIX), loads the core cdylib.
apps/linux/      GTK4 + libadwaita app in Rust (gtk4-rs), depends on the core crate directly.
```
**Rule:** all agent logic lives in the core. A UI layer may hold view state, timers and OS integration
(wallpaper setting, secure storage, notifications, login items), never decisions about what to make.

## Core responsibilities vs host responsibilities
| Core | Host (each app) |
|---|---|
| Keywords, settings, history, taste, spend (SQLite) | Native UI, accessibility, localisation of UI strings |
| Composing concepts (LLM), novelty, echoes, selection | Timers (native: NSBackgroundActivityScheduler / DispatcherQueueTimer / glib), wake/resume |
| Calling providers, network policy, cost estimates | Secure key storage, via the `SecretStore` callback |
| Decoding, validating, thumbnailing, per-display rendering | Display enumeration; setting wallpaper + lock screen per display |
| `is_due` / `next_due`, budget checks | Login item, notifications, tray/menu bar, updates |

## Data directory (passed in by the host; the engine creates it)
```
autopaper.sqlite3
images/<yyyy>/<id>.<png|jpg|webp>   original as delivered (validated); pruned by storage limit, never if liked
thumbs/<id>.jpg                     640 px wide; kept when the original is pruned (History still shows it)
renders/<id>-<w>x<h>.jpg            per-display render (new name per image: macOS ignores an unchanged URL); a cache,
                                    pruned first
```
Memory (concepts, embeddings, ratings) is **kept forever** even when image files are pruned: echoes and
novelty need years of history, and rows are tiny. Paths in the database are absolute. `image_bytes` in
`storage_usage` counts originals, thumbnails and renders; prune runs after every new wallpaper and on request,
deletes files before clearing paths (a file that can't be deleted stays tracked for the next prune), and never
touches the current wallpaper, a liked one, or the one just made (the host hasn't shown it yet). Image files no
generation owns (`<uuid>.<ext>` over an hour old: left by a call the host dropped mid-way) are deleted too, and so
are failed and refused rows older than **30 days** (they hold no files; kept that long for the CLI and support).
**The desktop's renders are never deleted while they're on the desktop (2026-10-06):** the wallpaper on the
desktop is the one most recently marked shown (state `desktop`, set by `mark_shown`; `current()` for older
databases), even after `delete_generation` or `clear_history` removed it from history — the OS shows its render by
path, so `clear_history`, `delete_generation` and prune all keep `renders/<desktop id>-*.jpg` (its original and
thumbnail do go). Once another wallpaper is shown, the next prune (or the cleanup after the next new wallpaper)
deletes renders that no wallpaper in history owns.

## Storage (SQLite, `PRAGMA user_version` migrations)
All tables STRICT; WAL, `foreign_keys = ON`, `busy_timeout` 5 s; multi-statement writes take an IMMEDIATE
transaction. Migrations only add and are never edited once shipped; a database from a newer
build opens without migrating. The engine holds the one connection behind a std `Mutex`. Version 2 (phase 2)
adds `generations.least_similar`; version 3 (round 4) moods; version 4 timings; version 5 Console runs; version 6 exact timing/run links; version 7 actual response models. A migration may carry a data step
(Rust, same transaction): version 3's makes the first mood.
- `moods(id, name UNIQUE NOCASE, position, surprise REAL, created_at)` (v3) — names unique case-insensitively in
  Rust (Unicode), NOCASE the ASCII backstop; positions 0..n. Always at least one (the store makes "My mood" if a
  database has none).
- `keywords(id, mood_id → moods ON DELETE CASCADE, text NOCASE, weight 'must'|'maybe'|'avoid', position,
  created_at, UNIQUE(mood_id, text))` — v3 rebuilt the table (the one migration that does: text was UNIQUE overall
  and SQLite can't drop that); case-insensitive uniqueness per mood is enforced in Rust; positions 0..n per mood.
  Keywords without a mood (an older build after a downgrade) or whose mood is gone join the active mood when the
  store opens (duplicates dropped).
- `timings(id, job 'concepts'|'images', provider, origin, model, width, height, steps NULL, seconds REAL,
  finished_at)` (v4) — one row per finished provider call, newest 50 kept per (job, provider, origin, model);
  index `timings_by_key`.
- `timings.run_id` (v6), `timings.answered_model` (v7) — new exact run association and actual response-model
  identifier; legacy values stay NULL. Estimate lookup retains the requested `model` key.
- `runs(id, started_at, status, record JSON text)` (v5) — full Console run record, time index; independently cleared/pruned without changing generations, taste or spend.
- `generations(id, created_at, trigger 'scheduled'|'manual'|'dislike_replace'|'echo_request', status
  'ok'|'failed'|'refused', title, summary, concept_json, prompt, text_provider, text_model,
  image_provider, image_model, width, height, image_path NULL, thumb_path, phash, embedding BLOB(f32 LE),
  embedding_model, echo_of NULL, echo_note, surprise, keywords_json, rating -1|0|1,
  rated_at, last_shown_at, shown_count, cost_microusd, error, hidden 0|1, least_similar 0|1, mood_id NULL)` —
  `echo_of` and `mood_id` (v3; index `generations_by_mood`) are soft references (no foreign key: echoes keep their
  original's id after it is deleted; a deleted mood's wallpapers keep its id and read `mood_name = None`). `hidden` marks rows
  cleared from history with memory kept. `least_similar` (migration 2; older rows read 0) marks a wallpaper
  that took the least similar candidate because nothing was novel enough (core-only: `StoredGeneration`, not
  the FFI `Generation`). Enum columns hold serde's snake_case names, no CHECK (new variants need no rebuild).
  Failed/refused rows have an empty embedding, appear in neither history nor memory, and are deleted by
  `prune` (and after each new wallpaper) once they are 30 days old.
- `taste(feature PK, likes REAL, dislikes REAL, updated_at)`
- `spend(month 'YYYY-MM' PK, microusd, images)` — UTC months.
- `settings(key PK, value JSON)` — one row per top-level field; missing or unreadable fields take their
  defaults one at a time; unknown keys (a newer build's) are kept.
- `state(key PK, value JSON)` — engine state: `consecutive_failures`, `backoff_until`, `last_slot_at` (when the
  schedule last restarted: the newest new wallpaper of any trigger, or a slot skipped over budget or cancelled; a
  database without it counts from its newest successful generation, and `slot_skipped_at` is only read from
  databases written before it), `slot_lead` (seconds before its due time the run that filled the last slot
  started: the next is due one interval after the due time, `next_start`), `desktop` (id of the generation most
  recently marked shown; may name a deleted or hidden one), `active_mood` (the active mood's id; missing or deleted
  → the first mood). `settings.surprise` is the active mood's: `settings()` reads it from the mood and
  `save_settings` writes it there (and to the settings row, for older builds).
- `clear_history(keep_memory: true)` hides every row from history, `current` and `lineage`, deletes the image
  files and keeps concepts, embeddings, ratings and taste (hidden rows are never echoed). `false` deletes
  generations and taste. Spend (money already spent) and keywords/settings stay either way. Either way the
  desktop's renders stay (see Data directory) and `current()` is `None` afterwards: hosts show an empty Now view
  while the desktop keeps its picture.

## Console run records
- Extend Engine/Store, not system logs or a second database. `RunRecord` includes original mood/keywords/
  Surprise, provider/model selections, finish state, generation ID, timing, known estimated cost and events.
  Start before preflight; success, failure, budget block, cancellation and interruption are distinct.
  Restart recovery and a dropped host future mark unfinished work interrupted, preserving known cost.
- Wrap only actual wallpaper-run provider dependencies at the existing HTTP boundary. Capture writer
  instructions/schema and actual POST payloads, model answers/usage, candidate checks/corrections,
  retries and painting parameters; GET model/history/download outcomes are also recorded.
- Omit authentication headers and image/base64 data; redact known credentials and sensitive JSON keys.
  Strip URL credentials/query/fragment. Non-JSON responses become byte-count summaries; truncation is
  labelled. Traces are bounded (about 512 KiB per run, 65,536 bytes per event, 255 events).
- Keep at most 200 runs / 30 days; preserve active runs while pruning. Summary pages omit events; selected
  run/report retrieves full already-sanitized JSON. Clear requires idle and never clears wallpapers/spend.
- Native date → runs → outcomes surfaces use existing navigation/model bridges. Copy/export is explicit
  and local; nothing is uploaded automatically. Older requests cannot be reconstructed.
- Real statistics (`core/src/engine.rs:342`, `core/src/store.rs:722`) use retained run outcomes/UTC days.
  Success/(success + failure) excludes blocked/cancelled/interrupted; average run duration includes finished
  successes/failures. Provider-call averages group exact linked timings by provider/job/actual response model;
  older samples are not inferred or backfilled. The 200-run/30-day and 50-timing-per-estimate-key limits remain.
- Integration: `core/src/engine.rs:318`, `core/src/engine.rs:1139`, `core/src/store.rs:678`,
  `core/src/model.rs:176`; task `tasks/2026-10/261007_console-budget-transparency.md`.

## Concept (LLM structured output, JSON Schema enforced)
`title` (≤ 60 chars, shown in menus), `summary` (1–2 sentences: memory, echoes **and the accessible
description of the wallpaper**), `setting`, `subject`, `elements[]`, `time_of_day`, `weather`, `season`,
`mood[]`, `palette[]`, `style`, `composition`, `keywords_used[]`, `wildcards[]`, `prompt`.
Prompt rules given to the model: a desktop wallpaper, landscape, no text/letters/logos/watermarks/UI,
calm areas where desktop icons sit, nothing from Avoid mentioned at all (image models misread negation).
The composer passes the person's locale so `title`/`summary`/`echo_note` read in their language; every other
field, and always the prompt, is English (the embedding model and the stemmer are English). The schema lists
fields plan-first (scene → keywords_used → prompt → title/summary); serde_json's `preserve_order` keeps that
order in the serialized schema.

**Prompt lessons from real renders (2026-10-05, ComfyUI on this Mac; `.scratch/examples/compare.jpg`):**
- Don't open the prompt with "Desktop wallpaper": Krea 2 Turbo painted a *desktop screenshot* (menu bar, dock,
  taskbar) twice. Describe the image as full-bleed, edge-to-edge artwork instead; "wallpaper" belongs in the
  instructions to the text model, not in the image prompt.
- Put medium and style early in the prompt: Z-Image Turbo largely ignores style words at the end, and varies little
  between seeds.
- Qwen-Image 2.1 always returns RGBA (alpha ≥ 236): the image pipeline flattens to RGB before JPEG.
- Speed per ~2 MP image on the M5 Max: Z-Image Turbo 42–74 s, Krea 2 Turbo 84–139 s, Qwen-Image 2.1 175–206 s.
- **Size per model (2026-10-06; user decision: each model's real limits, not one flat cap).** The macOS app's run
  asked Z-Image Turbo for 2544×1632 (a Retina 4112×2658 hint under the old 2560 px / 2048² cap) and VAEDecode
  failed on Apple silicon: "MPSGraph does not support tensor dims larger than INT_MAX". Cause (ComfyUI 0.36
  `slice_attention`): the VAE decoder's mid-block self-attention matrix is ((W/8)·(H/8))² elements for an 8× VAE,
  and ComfyUI slices it only when free memory runs short — so on a big Mac it's computed whole and must stay
  ≤ INT_MAX: (W/8)(H/8) ≤ 46,340, i.e. ≤ 2,965,760 px. Measured on the M5 Max with each model's real graph:
  Z-Image Turbo and Krea 2 Turbo decode 2048×1440 and fail at 2048×1456 and 2048×2048; 2544×1632 failed once and
  passed once (memory-dependent). Qwen-Image 2.1's VAE is 16×, so its card max (2752×1536, 2048²) decodes fine.
  ComfyUI's `capabilities(model)` now reads the model the workflow loads (bundled: the chosen model, else its
  own; a person's workflow: its loader's `unet_name`/`ckpt_name`) and looks it up in `comfyui::MODEL_LIMITS`
  (sources in its doc comment):

  | Model (file name contains) | Max side | Max pixels | From |
  |---|---|---|---|
  | Z-Image Turbo (`z_image_turbo`, `zimage_turbo`) | 2048 | 2,965,760 | Z-Image README "512×512 to 2048×2048 (total pixel area)", Turbo demo sizes ≤ 2048 a side; stepped down to the Apple silicon VAE bound |
  | Qwen-Image 2.1 (`qwen_image_2_1`) | 2752 | 4,300,800 (2400×1792) | model card's supported sizes 2048², 2400×1792, 2528×1696, 2752×1536 |
  | Krea 2 Turbo (`krea2_turbo`, `krea_2_turbo`) | 2048 | 2,965,760 | Krea 2 README "1k ~ 2k", `--width/--height 1024 ~ 2048`; stepped down to the VAE bound |
  | Krea 2 Raw (`krea2`, `krea_2`) | 2048 | 1,048,576 | README "trained to generate upto 1k resolution" |
  | anything else | 2048 | 2,088,960 (1920×1088) | conservative default |

  Sizes are multiples of 16 at the display's aspect (pixels capped by the display too); the renderer scales up per
  display. Real runs through the engine (CLI, Demo writing): Z-Image Turbo at a 4112×2658 hint → 2032×1312 in
  72 s, at 3840×2160 → 2048×1152 in 62 s; Krea 2 Turbo (a person's workflow) at 4112×2658 → 2032×1312 in 140 s;
  Qwen-Image 2.1 (a person's workflow, 25 steps) at 4112×2658 → 2576×1664 in 536 s (~20 s a step). **Open item:**
  that is 64 s inside ComfyUI's 600 s job timeout on the fastest Apple silicon there is; a slower Mac would time
  out every time at Qwen-Image 2.1's full size. Options for the user: a longer ComfyUI timeout (or one that only
  fires while the job makes no progress), or a time-based cap for slow models. **Resolved (round 4, user decision
  2026-10-06):** progress-based — no event for a stall window, or past max(30 min, 3 × the learned estimate).
  VAEDecodeTiled (tile 512, overlap 64) also gets past the bound (2544×1632 in ~2 min 20 s) but isn't used: the
  bundled workflow stays the official template, and the renderer upscales anyway. A CUDA server could paint
  more with the 8× models; the provider can't tell the server's GPU when the size is chosen, so the bound applies
  everywhere.
- Licences: Z-Image Turbo Apache-2.0 (the bundled ComfyUI default); Krea 2 Community License (outputs owned; revocable,
  content-filtering and AUP terms); Qwen-Image 2.1 Qwen Research License (outputs belong to the user per Qwen) — the
  person may choose any model in their own ComfyUI (all three have a bundled workflow since 2026-10-06); AutoPaper's
  defaults stay permissive (Z-Image Turbo is the default).

## Composing and selection
1. Build the request: Musts (all required), Maybes (use some; vary across candidates), Avoids (never),
   Surprise σ ∈ [0,1], taste hints (top liked/disliked features with evidence), and the ~20 most recent
   summaries ("don't repeat these").
2. Ask for **N = 4 candidates** in one call. Temperature ≈ 0.4 + 0.8σ where the model supports it.
   Wildcards allowed: 0 (σ < .25), ≤1 (< .5), ≤2 (< .75), ≤3 (otherwise); above .75 keywords are loose inspiration.
3. Deterministic checks per candidate: every Must appears in `prompt` (word-stem match), no Avoid
   appears in prompt, title or summary (stem match), non-empty title/summary/prompt, prompt ≤ 2500 chars.
4. Score = novelty × (1 − 0.3σ) + taste × (1 − 0.6σ) + U(0, 0.3σ) (novelty = 1 − max penalty; jitter from
   the engine's seeded RNG). Pick the best valid novel candidate.
5. If none is novel enough: re-ask (at most twice) with the nearest past summaries named as "too close"
   (accumulated, ≤ 8); then take the least similar valid one. **Narrow keywords (user decision, phase 2):** once
   there is a valid candidate to fall back on, a retry that doesn't lower its max penalty ends the retries
   (asking again only costs); retries while no answer has had a valid candidate are unchanged (unreadable
   answers, missed Musts). A wallpaper that took the fallback is stored with `least_similar = 1`.
   `keywords_are_narrow()` is true when at least **3 of the last 5** successful non-echo wallpapers (any trigger;
   cleared history counts) took it; hosts then show "Your keywords are narrow, so new ideas are getting hard to
   find. Add some Maybes or raise Surprise for more variety." It turns false as new ideas come back. No valid
   candidate at all → `InvalidResponse` and a failed row.
6. A refusal (content policy, from the text or the image provider) gets one recompose that asks for a gentler,
   plainly described scene; a second refusal fails as `Refused`.

## Novelty (memory)
- Embedding text = title + summary + setting + subject + elements + time/weather/season + mood + palette, as
  plain sentences without labels (`embed::concept_text`); style, composition and prompt are left out.
- **candle 0.11 + BAAI/bge-small-en-v1.5** (CLS pooling, L2-normalised, 512 tokens) inside the core, so memory
  works offline and identically on every platform. Without the model files the engine falls back to
  `HashingEmbedder` (stems + bigrams, 384 buckets; logged), which only catches near-verbatim repeats.
  `embedding_model` is stored; after a model change the engine re-embeds memory from stored concepts lazily,
  once, before its next novelty check.
- For each past generation: `penalty = cosine × w(age)`, `w = 1` within the **quiet period Q**
  (setting: 1 month · 3 months · **6 months** · 1 year · 2 years), then halving every Q/2.
  Too similar ⇔ `max penalty ≥ θ`. **θ = 0.79** for bge-small (calibrated: below every near-duplicate, 0.811+,
  above 13 of 14 related pairs; a related pair whose shared keywords describe the whole scene scores 0.846 and
  reads as a repeat). The hashing fallback has its own calibration (θ = 0.70, echo band 0.40–0.85, measured on
  the same pairs); `novelty::Calibration::for_model` picks by model id. Numbers: `core/src/novelty.rs`,
  `core/tests/novelty_calibration.rs`.
- After generation, a 64-bit perceptual hash is stored; a near-duplicate of a recent image is logged
  (not regenerated — that would bill twice).

## Echoes
- Eligible originals: status ok, not disliked, not cleared from history, older than Q; no echo of the same
  lineage within Q. Weighted: liked ×3, unrated ×1, × (1 + years, capped at 3).
- Chance per scheduled wallpaper: Off · Rarely 5% · **Sometimes 12%** · Often 25% (rolled only when an original
  is eligible). Also **Make an Echo** on any past wallpaper in History (explicit request; ignores Q and the
  lineage rule). A scheduled echo that yields no usable candidate becomes an ordinary new wallpaper.
- The composer gets the original concept, how long ago it appeared, and 2 axes (σ < 0.5) or 3 chosen by the
  core (weather · time of day · season · passage of time: decay or renewal · viewpoint/distance ·
  era/medium); axes the original pins down (time, weather, season) are twice as likely. Avoids still apply;
  Musts and Maybes don't (it's the old idea's echo).
- Valid echo: cosine to the original within the band (**0.72 ≤ cos < 0.93** for bge-small: holds every
  calibrated echo, excludes every copy and unrelated pair — but cosine can't tell an echo from a rewording of
  its original), at least one requested axis visibly changed its field (time_of_day, weather, season, style
  for medium, composition for viewpoint; by `text::feature_key`), and novel against everything outside its
  lineage. Stored with `echo_of` and a one-line `echo_note`; when the model leaves it out the engine writes
  "Echo of “title” (age)".

## Taste
- Features: stemmed keys (`text::feature_key`) of setting, subject, elements, palette, mood, style,
  time_of_day, weather, season, and keywords used. Hints and "What AutoPaper has learned" show them in the
  words the most recent rated wallpaper used ("warm ivory", not the key "warm ivor").
- Like → `likes += 1` per feature; Dislike → `dislikes += 1`; changing a rating undoes the old one; counts decay
  by half per year (lazily; a clock set back never grows them).
- Concept taste = mean over known features of `((likes+1)/(n+2) − 0.5) × min(1, n/3)`.
- Hints to the LLM: up to 6 liked (mean ≥ .65, n ≥ 2) and 6 disliked (mean ≤ .35, n ≥ 2) features. Because
  counts decay continuously, two ratings fall just under n = 2 at once: in practice a hint takes three.
- Avoid keywords are hard rules; dislikes are soft. **Dislike** can replace the wallpaper right away
  (setting "Replace wallpapers I dislike", on by default, within budget).

## Scheduling and budget
- Cadence: Every hour · 3 h · 6 h · 12 h · **Every day** · Every week · Only when I ask.
- `next_due(now)` / `next_start()` / `run_if_due(observer)`; hosts call on their native timer and on wake.
  **On time (round 4):** `next_start = next_due − lead`, lead = the writing + painting estimates for the current
  providers and display (0 until something is recorded), at most half an interval, never before a backoff ends;
  `run_if_due` is due from `next_start`, and a run that started early fills the slot due at `next_due` (state
  `slot_lead`), so early starts don't drift the schedule. Over budget, cancelled and needs-setup runs fill their slot
  the same way. **Every new wallpaper
  restarts the schedule (user decision, phase 2):** it counts from the newest successful new wallpaper of any
  trigger — Scheduled, Manual ("New Wallpaper Now"), DislikeReplace, EchoRequest — or from a slot skipped over
  budget (`last_slot_at`); revisits (a liked one brought back) and failed attempts don't move it. Hosts re-read
  `next_due` after every generation. A clock set back past it moves it to now, so it waits at most one interval.
- Monthly cap (USD, default $5, UTC months): cost estimated per call from a versioned price table per
  provider/model/size (`pricing`, as of 2026-10-05; Gemini priced at the size it actually returns) + text tokens
  reported by the API; local providers and OpenAI-compatible servers cost 0 and always fit the budget. Each paid
  call is added to spend as soon as it answers, so failed, cancelled and dropped attempts count (an answer the
  composer can't read too: providers pass it on as text with its usage, and the engine asks again). Shown in
  Settings, labelled "estimated".
- Over budget: the slot counts as filled and returns `BudgetReached`; the current wallpaper stays in
  place regardless of provider-failure fallback. The prospective gate runs before key lookup or network
  requests. `budget_status()` exposes exact microUSD spending/next estimate and a notice linking to Budget.
  Native hosts refresh it after settings/runs and at the UTC month boundary, including while paused/manual.
  Any failure backs off 10 → 20 → 40 → 60 min from when it
  failed, or longer when a rate limit's `Retry-After` asks (a missing or rejected key too, so hosts never spin).
  A cancel isn't a failure (no backoff), but **a cancelled scheduled run fills its slot** (2026-10-06), like one
  skipped over budget, so a host timer doesn't restart it at once; that holds when it was cancelled while waiting
  for its turn too. The first failure in a row that is transient, a refusal or an unusable answer revisits a liked
  wallpaper (`Offline` / `ProviderFailed`); later ones, and problems the person must fix, return the error.
  A painting ComfyUI couldn't run (`needs_setup`, 2026-10-06) is never revisited and fills its slot (see ComfyUI).
  Success clears the backoff.

## Image pipeline
Pick the provider size closest to the primary display's aspect, largest available (free-size providers: no
more pixels than the display; on their step grid, the closest aspect among sizes with at least 95% of the pixels
the caps allow at the exact aspect, aspects within 0.1% decided by more pixels — ComfyUI with Z-Image Turbo:
16:9 → 2048×1152, a 4112×2658 Retina hint → 2032×1312) → download/decode with limits (≤ 50 MB, ≤ 64 MP, ≤ 16384 px a side,
PNG/JPEG/WebP only, sniffed not trusted, EXIF orientation applied) → original + thumbnail + pHash (atomic
writes) → per display: centre-crop to the display aspect, Lanczos3 scale to native pixels, light unsharp mask
only when upscaling > 1.25×, JPEG q90 without metadata. v1 shows one wallpaper on all displays, rendered per
display. The engine doesn't enumerate displays: it aims at **3840×2160 (16:9)** until the host passes its primary
display's size (`set_display_hint(w, h)`, at launch and when displays change). Decoding,
thumbnails and embeddings run on the runtime's blocking pool.

## Providers (plugins: one file each + one registration line)
- `TextProvider { compose(ComposeRequest) -> candidates + usage; list_models() }`
- `ImageProvider { capabilities(model); generate(ImageRequest) -> bytes, mime, usage; list_models() }`
- v1 (verified against official docs 2026-10-05; reference: `docs/research/providers.md`):
  - **OpenAI** — text: Responses API, strict `json_schema`, `store: false`, default `gpt-6-luna`
    (reasoning effort none/low). Image: `gpt-image-2.5-flare`, base64 only, no seed/negative prompt,
    up to 3840×2160 (>2560×1440 "experimental"), ~21:9 3840×1648; Standard = medium, High = high quality.
    Refusal = `error.code "moderation_blocked"`. Up to ~2 min per image.
  - **Google Gemini API** (AI Studio key, `x-goog-api-key`) — `generateContent` (documented shapes; the
    newer Interactions API's response shape is unverified). Text `gemini-3.5-flash-lite` with
    `responseSchema`; image `gemini-3.1-flash-image` (Nano Banana 2), `imageSize` "2K"/"4K" (uppercase K),
    16:9 4K = 5504×3072, 21:9 = 6336×2688, no 16:10 (crop from 3:2). Blocks arrive as
    `promptFeedback.blockReason` / `finishReason` IMAGE_SAFETY|NO_IMAGE. SynthID watermark. Imagen is gone.
  - **Ollama** — text only (`/api/chat` with `format: <schema>`, port 11434; `/api/tags`). Image generation
    was removed from Ollama in 2026-07; never offer it for images.
  - **OpenAI-compatible** — text (`/v1/chat/completions` + `response_format: json_schema`; LM Studio on
    1234) and/or image (`/v1/images/generations`; LocalAI 8080, stable-diffusion.cpp `sd-server`).
  - **ComfyUI** — image: `POST /prompt` → poll `/history/{id}` (or `/ws`) → `GET /view`; port 8188.
    A workflow template plus a node-input map (prompt, negative, width, height, seed), since node IDs vary.
    **One bundled workflow per model (2026-10-06, after the user's bug):** the old picker listed every diffusion
    model and only swapped `unet_name`, so Qwen-Image 2.1 ran with Z-Image's encoder/VAE and KSampler failed ("Given
    normalized_shape=[4096] … size[1, 95, 2560]"), which the app called retryable. Now `core/resources/comfyui/` has
    Z-Image Turbo (default), Krea 2 Turbo and Qwen-Image 2.1, each the graph that painted the website examples, with a
    map declaring `name` and `requires` (diffusion model, text encoder, VAE, custom nodes). A chosen model picks its
    own workflow (never another's); blank = Z-Image Turbo; a model without one → `InvalidInput` `Other` from
    `ImageProvider::check_model`, which the engine calls in `prepare` (before an idea is paid for, nothing recorded).
    `list_models` (bundled) does one `GET /object_info` and lists a model only if its declared files are among
    UNETLoader/CLIPLoader/VAELoader's choices, every node class of its graph exists, and every other fixed combo
    value (CLIP type, sampler) is offered — named in words, default first, then by name. A person's own workflow
    keeps the old behaviour (its loader decides; its loader's choices listed). Krea 2 Turbo drops the website graph's
    `ConditioningKrea2Rebalance` (third-party Rebalance-Pack): Comfy-Org's own template doesn't use it, and an A/B at
    768×432 with a composer-style prompt painted as well without it; stock nodes only. Failures name model and node:
    "ComfyUI rejected the workflow for Qwen-Image 2.1: …", "ComfyUI couldn't paint with Qwen-Image 2.1: the KSampler
    node (8) failed: …" (still `InvalidResponse`, no FFI change). `engine::needs_setup` (ComfyUI painting +
    `InvalidResponse`/`InvalidInput`): a scheduled run isn't covered by a revisit and **fills its slot** (next try one
    interval later, or when asked) — no 10-min retry loop recomposing and reloading models. Live (CLI, Demo writing,
    512×288, M5 Max): Z-Image Turbo 4.1 s, Krea 2 Turbo 20.1 s, Qwen-Image 2.1 46.1 s (25 steps, incl. model loads).
    macOS: Providers → Painting → ComfyUI has a Workflow picker (AutoPaper's · the file's name · Your Own…/Choose
    Another…) above Model (AutoPaper's: the menu; own: a read-only "Model: … (from your workflow)" line); the Now view
    says "ComfyUI couldn't paint with Qwen-Image 2.1." + **Check ComfyUI in Settings** (opens Providers), telling a
    painting failure from an unusable idea by the last progress stage (Painting…).
    **Round 4:** failures are `PaintingFailed { provider, model (in words), detail }` (not `InvalidResponse`): a /prompt
    rejection, a node that failed, a run that saved no image, and a model AutoPaper has no workflow for (from
    `check_model`, before anything is spent: it fills a scheduled slot too). Hosts say one line + link to Providers.
    **Progress (round 4):** `generate` opens `/ws?clientId=<id>` before POST /prompt with the same `client_id` (ComfyUI
    sends a job's `progress` only to the client that queued it) and reads it alongside the /history polls; /history
    stays the authority on how the job ended, so without the socket (https server, broken connection) the job is
    still followed and finished, only without step progress and stall detection. Steps → `ImageRequest::progress`
    as done/total (total = Σ `steps` inputs of the graph's samplers, `ImageProvider::steps`: 8 for Z-Image Turbo and
    Krea 2 Turbo, 25 for Qwen-Image 2.1). An `executing` node null / `execution_*` event polls at once. **Learned
    timeout:** a running job fails (`ProviderUnavailable` `TimedOut`, job stopped) after no event for max(3 min, 4 ×
    its slowest step) after a sampler step, or 10 min while a node without steps runs (loading, encoding, decoding);
    queued behind someone else's job is never a stall; overall ceiling max(30 min, 3 × the learned estimate) from the
    start (replaces the flat 600 s, the open item about Qwen-Image 2.1 on slower Macs).
    Sizes: multiples of 16 within the limits of the model the workflow loads (`MODEL_LIMITS`; see Prompt lessons). A job given up on (a stall or
    the ceiling, a failed poll, or the engine dropping the call on `cancel`) is stopped with `POST
    /api/jobs/{id}/cancel` (ComfyUI since 2026-06, PR #14493; 0.36.0 has it: interrupts that job only, or
    dequeues it); a server without that route
    (404/405) only gets `POST /queue {"delete": [id]}` — never `/interrupt`, which stops anyone's job.
  - Prices (per image, 2026-10-05): gpt-image-2.5-flare 3840×2160 medium $0.026 / high $0.100;
    Nano Banana 2 2K $0.101 / 4K $0.151. Neither has an upscale endpoint; Gemini 4K covers 5K displays.
  - Privacy: OpenAI Responses get `store: false`. Gemini `generateContent` keeps no application state and has
    no `store` field, so none is sent (only the Interactions API stores; it would need `store: false` if ever
    adopted). Gemini **free-tier** text may be used for training and human review — Settings and PRIVACY.md say
    so.
- Keys come from the host's `SecretStore` (`openai.api_key`, `google.api_key`, `openai_compatible.api_key`);
  never logged, persisted by the core, or shown in errors (`HttpRequest`'s Debug redacts credential headers;
  error details pass through `net::redact`).
- As built: Gemini text uses `responseMimeType` + `responseSchema` (the documented, now-deprecated path; the
  JSON-Schema `responseFormat` path is a one-function switch) — unverified live, like the rest of the hosted
  calls until the ignored smoke tests run with real keys. Demo is seeded from the engine's RNG (reproducible
  runs); its English says "a"/"an" by sound and names each thing once. Ollama is sent `think: false`. ComfyUI
  jobs are stopped when they stall or pass their ceiling (see ComfyUI above); a live test proved the cancel on the
  user's ComfyUI 0.36 (idle 1.5 s after it, history `execution_interrupted`).

## Network policy (from AudioPaper's untrusted-response hardening)
- Hosted providers: https to their API host only; images requested as base64 where possible.
- Local providers: the person's base URL; plain http allowed only to loopback / private / link-local /
  `.local` addresses; https anywhere for OpenAI-compatible services.
- No credential headers across hosts on redirect (nor across a scheme or port change); redirects only to the
  same host, never https → http; ≤ 3 redirects; size caps; timeouts (text 60 s, image 240 s);
  `Retry-After` honoured (by the schedule's backoff); no cookies; User-Agent "AutoPaper/<version> (<client>;
  +repo URL)". An image URL an OpenAI-compatible server returns is fetched only from the server's own origin
  (scheme, host and port).
- Text from providers (concept fields, model names) is stored without control characters; the CLI strips them
  from everything it prints, so a server can't drive the terminal.
- Hosted APIs follow the system proxy; local destinations are always reached directly.
- The client reports timeouts and transport failures as `Offline` (it doesn't know the provider). Providers
  that know better map them: a failure after (nearly) the request's whole timeout (1 s slack,
  `providers::used_whole_timeout`) was a timeout — `ProviderUnavailable` with reason `TimedOut` (OpenAI and
  Gemini detail "timed out": the provider is slow; a scheduled run revisits with `ProviderFailed`, not
  `Offline`; local providers "didn't answer within N s"); other failures stay `Offline` for hosted APIs
  (connection, DNS, the 10 s connect timeout) and become `NotRunning` ("isn't running at …") for local ones.
  HTTP 408/504 are `TimedOut`, 500/502/503 `ServerError`, a ComfyUI job stopped on the server `Stopped`.
- PRIVACY.md and NETWORK.md list every host, request and stored item, and change with the code.

## Service availability before wallpaper generation

- Extend the existing provider traits with read-only `check_available`: model-list requests by default,
  ComfyUI `GET /object_info` even for custom workflows without a recognized loader. Demo stays local.
  Engine checks both roles independently in parallel, with an 8-second timeout and prompt cancellation,
  after the budget gate and before composing. Validate the chosen local workflow after both checks,
  still before paid work. Console records both `service_check` outcomes and actual GET requests.
- Native manual, dislike replacement and explicit echo actions use `generate_or_revisit` /
  `make_echo_or_revisit` returning `Shown`; scheduled actions already return `Shown`. Strict generation
  APIs retain their error contract. Progress begins at `CheckingServices`.
- An availability failure returns `ServicesUnavailable` with the latest successfully stored usable image
  in the active mood captured at run start. This applies even when an explicit echo's source belongs
  elsewhere, and independently of the other scheduled-failure preference. Reuse mood history in newest
  order, skip disliked/missing/corrupt originals, decode under existing image limits on a blocking thread.
  If none remain, return the original service error and preserve the current wallpaper.
- A saved image is a revisit: Console remains Failed, spending stays zero, no new generation is stored,
  and native new-image announcements/notifications are suppressed. Repeated scheduled failures retain
  normal backoff and can revisit on every failed check; recovery resets it normally.
- Integration: `core/src/engine.rs:500`, `core/src/engine.rs:2021`, `core/src/providers/mod.rs:151`,
  `core/src/providers/comfyui.rs:540`, `core/tests/engine.rs:2393`.

## FFI surface (UniFFI 0.31.2, proc-macros, async on tokio; C# via uniffi-bindgen-cs v0.11.0+v0.31.0)
`Engine::open(EngineConfig{data_dir, model_dir, locale}, SecretStore)`; keywords (list/add/set weight/
reorder/remove); `settings()` / `update_settings()`; `current()`, `history(filter, limit, offset)`,
`generation(id)`; `async generate(trigger, ProgressObserver?)`; `async generate_or_revisit(trigger, observer)`; `async run_if_due(observer)`; `next_due()`;
`rate(id, rating)`; `async make_echo(id)`; `async make_echo_or_revisit(id, observer)`; `render_for_display(id, w, h) -> path`; `revisit_liked()`;
`spend_summary()`; `taste_summary()`; `async test_provider(kind)`; `async list_models(kind)`;
`storage_usage()`; `prune()`; `clear_history(keep_memory)`; `describe(id) -> spoken description`;
`set_display_hint(w, h)`; `memory_status()` (embedding model, `reduced` when on the hashing fallback, why);
`keywords_are_narrow() -> bool`; `has_echoes(id) -> bool` (2026-10-06: `lineage(id)` would list more than this
one; one query, so hosts ask per History item instead of a field on every `Generation`). Free functions
(2026-10-06): `prices_as_of() -> String`, `default_base_url(kind) -> String?` (Rust: `registry::default_base_url`
→ `Option<&'static str>`; exported under that name from `default_base_url_for_host` with
`#[uniffi::export(name = …)]`), `default_model(kind, job) -> String` (empty when there's no fixed default),
`surprise_band(surprise) -> SurpriseBand` (Faithful/Fresh/Adventurous/Wild; `wildcard_limit` derives from it),
`secret_account_for(selection)`. `ProviderJob` lives in `model.rs` now (same name and path for hosts).
**Round 4 (additive except the new enum cases):** moods — `moods()`, `active_mood()`, `create_mood(name, copy_from?)`,
`rename_mood(id, name)`, `delete_mood(id)`, `set_active_mood(id)`, `move_mood(id, to_position)`,
`add_mood_keyword(mood_id, text, weight)`, `set_mood_surprise(mood_id, surprise)`, `history_by_mood(filter,
mood_id?, limit, offset)`; records `Mood {id, name, position, surprise, created_at, keywords, active}`,
`Generation.mood_id?/mood_name?`; performance — `estimate(selection, job, width, height) -> u32?` (0 × 0 = the
size it would ask for), `next_start() -> i64?`, `set_progress_detail_observer(observer?)` with the foreign trait
`ProgressDetailObserver { on_progress_detail(ProgressDetail) }` and record `ProgressDetail {stage, fraction?,
seconds_left?}`; error `PaintingFailed { provider, model, detail }`; `InvalidInputReason` + `MoodNameEmpty`,
`MoodNameTooLong`, `DuplicateMoodName`, `LastMood`. The detail observer is a second, engine-wide trait rather than a
method on `ProgressObserver` because UniFFI 0.31 exported traits can't have default methods (`uniffi_macros`
refuses them): a new method would break every host's observer, while this is opt-in. Rust-only:
`Engine::set_demo_delay`.
**Console (2026-10-07):** `runs(limit, offset)`, `run(id)`, `run_report(id)`, `clear_runs()`,
`console_statistics()` and `budget_status()` expose local bounded diagnostics/statistics and prospective
monthly budget status through the same bridges. See Console run records above; not a second logging service.
Errors are typed enums (hosts localise). **Typed reasons (2026-10-06):** `InvalidInput { reason:
InvalidInputReason, detail }` (KeywordEmpty, KeywordTooLong, TooManyKeywords, DuplicateKeyword, AddressMissing,
AddressNotAllowed, AddressInvalid, DisplaySizeInvalid, NoModels, WorkflowNeedsPrompt, WorkflowNotApiFormat,
WorkflowInvalid, NothingToEcho, Other) and `ProviderUnavailable { provider, reason: ProviderUnavailableReason,
detail }` (TimedOut, NotRunning, ServerError, Stopped, Other); `detail` stays, English, for logs — hosts word the
reason and never match on `detail`. Threading: sync methods block the calling thread (`open`,
`render_for_display`, `prune`, `clear_history`, `delete_generation`, `rate`, `history`, `storage_usage` are
marked **Blocking**), so hosts call them off the UI thread; async methods do SQLite work on the thread that polls
them and image/embedding work on the blocking pool, and `ProgressObserver` is called on the polling thread —
not necessarily the UI thread, so hosts marshal. Rust hosts poll async methods inside a tokio runtime. A host
callback that throws (UniFFI makes it a panic) is caught: an observer is ignored, a secret store reads as no key.
Progress stages: Composing → CheckingMemory → Generating → Downloading → Rendering → Done. `generate` and
`make_echo` don't set anything: the host renders, sets the wallpaper, then calls `mark_shown`. `cancel` stops
the generation promptly (2026-10-06): a provider request under way is raced against a `watch` of the cancel count
and dropped (its connection closes; ComfyUI's job is cancelled on the server by a drop guard), requests waiting for
their turn stop waiting, and stage boundaries still check (`Cancelled`, no failed row). An answer that arrives
with the cancel is taken first, so its reported cost is recorded; a paid request dropped mid-way may still be
billed by the provider but its cost is unknown and isn't counted.
`describe` is title, summary and echo note as sentences, in the language they were written in; it doesn't
include the rating (phase 2: hosts say "Liked" / "Disliked" in their own language). Rust-only (not exported):
`Engine::open_with(config, secrets, Deps{clock, http, embedder, rng_seed})`, `compose_preview` (the CLI's dry
run), `stats`.

## Native surfaces (per platform HIG)

User preference (2026-10-07): native list/form defaults throughout. Use the toolkit's appropriate sidebar, list,
outline and form/preference controls to determine row spacing, typography, selection, separators and icon sizing.
Left-panel icons remain; native icon/label controls supply their geometry. Native form sections/settings groups
provide headings and spacing, replacing redundant app-drawn rules/header bands. Preserve semantic row data,
editing/actions/keyboard/AX and functional list viewport/image geometry. See `docs/app-spec.md`.

| | macOS | Windows 11 | Linux (GNOME HIG; KDE works) |
|---|---|---|---|
| Always-on | Menu bar extra, **menu** style | Notification-area icon + context menu | Background portal + window; SNI tray on KDE only |
| Main window | Now · Moods · History (Moods = list › detail) | NavigationView + Mica, Settings-style cards | AdwApplicationWindow + view switcher |
| Keywords | Rows with Must/Maybe/Avoid segmented control | Same choice as RadioButtons (the Toolkit Segmented exposed no position/selection state to UI Automation — accessibility deviation, see KeywordsPage.xaml) | Same, AdwToggleGroup |
| Like/Dislike | Menu, window, App Intents, AppleScript | Window, tray menu, toast buttons | Window, notification actions |
| Settings | Panes: General · Providers · Accounts · Memory · About | Settings page | AdwPreferencesDialog |
| Keys | Keychain (auto-save) | Credential Manager / PasswordVault | Secret Service (oo7/libsecret) |
| Wallpaper | NSWorkspace per screen (lock screen follows) | IDesktopWallpaper + lock screen API | XDG Wallpaper portal, set-on background/lockscreen/both |
| Login | SMAppService | MSIX StartupTask | Background portal autostart |
| Updates | Sparkle | MSIX / winget | Flatpak |

### Setting up local models: Brew Browser (user request, 2026-10-05)
When a local provider is chosen in Settings → Providers, the pane helps people install it, through Brew Browser
(brew-browser.zerologic.com, the user's Homebrew app) where Homebrew can do it. Facts (brew-browser `bundles.json`,
origin/main 2026-09-15; Homebrew formula API 2026-10-05):
- **Local LLMs** bundle (`local-llm`) = `ollama` (formula; Homebrew bottles for macOS **and** Linux arm64/x86_64) +
  Open WebUI (cask, macOS only). **Image Generation** bundle (`image-gen`) = `comfy` + `draw-things` (casks, Apple
  silicon only → macOS only).
- Brew Browser runs on macOS (native app `com.zerologic.brew-browser-native`, macOS 26; cross-platform Tauri build
  `com.zerologic.brew-browser`, macOS 13+) and **Linux** (Tauri build: .deb/.rpm/.AppImage, Ubuntu 22.04+).
- **Deep links (added by the user to Brew Browser, 2026-10-05; shipping in its next build):**
  `brewbrowser://bundle/<id>` focuses or launches Brew Browser, goes to Bundles and opens that bundle's detail.
  Navigate-only — it never installs by itself. An unknown id lands on the Bundles list. Both shells handle it; macOS
  cold and warm start; on Linux cold start only for now (routing to an already-running window needs Brew Browser's
  single-instance plugin, a noted follow-up there).
Per platform:
- **macOS** (Ollama and ComfyUI): under the address field, a footnote "Not installed? Brew Browser can set it up:
  its <Local LLMs | Image Generation> bundle installs <Ollama | ComfyUI>. Models are separate downloads." plus a button:
  **Open in Brew Browser** when something handles the scheme
  (`NSWorkspace.shared.urlForApplication(toOpen: URL(string: "brewbrowser://bundle/local-llm")!)` non-nil) → opens
  `brewbrowser://bundle/local-llm` or `brewbrowser://bundle/image-gen`; otherwise **Get Brew Browser…** (opens the
  website). Older Brew Browser installs without the scheme fall back the same way. Shown for local kinds, emphasised
  when Test fails with `ProviderUnavailable` ("isn't running at …").
- **Linux**: the same for **Ollama only** (an `AdwActionRow` with a button): if `gio::AppInfo::default_for_uri_scheme
  ("brewbrowser")` exists, launch `brewbrowser://bundle/local-llm` (OpenURI portal inside Flatpak); else open the
  website. Until Brew Browser's single-instance follow-up lands, a link sent while it's already running may only focus
  it — the footnote names the bundle, so the person can still find it. ComfyUI links to ComfyUI's own install
  instructions instead (the bundle is macOS-only).
- **Windows**: Homebrew doesn't run natively, so link each project's own installer instead (Ollama's Windows download /
  `winget install Ollama.Ollama`; ComfyUI Desktop for Windows) — no Brew Browser mention.

## Performance history and progress (user decisions, 2026-10-06; built in core round 4)
"Whatever the largest is that the model will do. We can determine pretty well how well the machine we're running on
will perform though over time. Will we keep that info?"
- **Size:** local models always paint at the largest size they support (`comfyui::MODEL_LIMITS`); no quality-based
  downscaling for ComfyUI. The pipeline upscales per display.
- **Record:** every finished provider call stores a timing row: job (concepts/images), provider kind, server origin
  (local servers can be other machines), model, width × height, steps (when known), seconds, finished_at. Kept local,
  never sent. Estimates use a recency-weighted median (half-life ~30 days) of comparable rows, scaled by pixels for
  other sizes; nothing recorded → no estimate (never a made-up number).
- **Progress:** ComfyUI reports step progress (`/ws` `progress` messages value/max, or polling); the engine turns it,
  plus the learned timing, into a fraction and seconds left. Hosts get it through the observer (an additive callback
  carrying stage + optional fraction + optional seconds left) and show a determinate ring and "about 6 minutes left"
  in the stage capsule; hosted providers stay indeterminate unless an estimate exists.
- **Timeouts:** a local job fails only when it stops making progress for a stall window (e.g. 3 minutes without a new
  step) or runs past max(30 minutes, 3 × its estimate). Hosted providers keep their request timeouts.
- **Estimates in Settings:** `estimate(selection, job, width, height) -> Option<seconds>` so pickers can say "About 9
  minutes per wallpaper on this Mac" next to a model.
- **On time:** a scheduled wallpaper starts early by its estimated duration, so it's ready when due (`next_due` stays the
  due time; a new `next_start` or the host's timer uses due − estimate).
- **Typed painting failure:** a provider that ran but couldn't paint (ComfyUI node error, model/workflow mismatch)
  reports a typed error naming provider and model, so every host can show one line + a link to Providers (rule 6a)
  without guessing from the last stage.

**As built (core round 4, `core/src/perf.rs`, `engine.rs`, `providers/comfyui.rs`):**
- Timings: `perf::Timing` rows for every provider call that answered (compose calls and paintings; failed or
  cancelled calls aren't recorded), measured with tokio's clock around the call. Origin = the server's origin for
  Ollama, ComfyUI and OpenAI-compatible (`http://127.0.0.1:8188`), empty for hosted and Demo. Model = the one asked
  for with the provider's default resolved (not the answering alias), so the next call finds it.
- Estimate: weighted median, weight 0.5^(age/30 days); each row scaled by target/recorded pixels and, when both
  sides know them, steps; exact half-weight boundary → mean of the two middle values. `Engine::estimate` rounds up
  to whole seconds. Engine-internal estimates use a Demo built without touching the seeded RNG.
- Progress: `perf::PaintTracker`. Before any step: elapsed/estimate (≤ 0.95), seconds left = estimate − elapsed
  (`None` once past it). Once steps come: the fraction at the first step is the base and steps fill the rest (never
  backwards, ≤ 0.99 until the picture arrives); seconds left = remaining steps × the pace measured between steps in
  this run (needs two step reports), else the estimate. The engine reports on each step and on a 1 s tick when the
  rounded percent or seconds changed; stages report `fraction/seconds_left = None` (Done: 1.0 / 0). The observer is
  engine-wide (`set_progress_detail_observer`), not a method on `ProgressObserver` (UniFFI can't default trait
  methods; see FFI surface).
- Timeouts, typed failure, early start: see ComfyUI and Scheduling above. Hosted providers keep their request
  timeouts; Ollama/OpenAI-compatible text keep theirs (180 s): the learned timeout is ComfyUI's (the only local *job*).
- Demo slow mode (`AUTOPAPER_DEMO_DELAY_SECS=N`, 1–3600, read by `Engine::open`; Rust: `set_demo_delay`): a
  quarter of N writing, the rest painting as 8 reported steps — so VMs can show the ring and time left without
  ComfyUI; its timings make Demo estimates appear after one run. Off by default.

## Moods (user request, 2026-10-06; core built in round 4)
"I'd like to experiment with different scenes but there's no way to easily switch moods while preserving the old set."
- A **mood** = a name + its own keywords (with weights and order) + its own **Surprise**. Exactly one mood is active;
  everything else (cadence, providers, budget, quiet period, echoes setting) is global.
- Store: `moods(id, name UNIQUE NOCASE, position, surprise, created_at)`; `keywords.mood_id` (FK, cascade on mood
  delete); active mood id in state; `generations.mood_id` (nullable; the mood a wallpaper was made under). Migration:
  existing keywords + Settings.surprise become the first mood, named from its first two keywords ("Rain, Beach"), else
  "My mood".
- Engine API (additive): `moods()`, `active_mood()`, `create_mood(name, copy_from: Option<id>)` (Duplicate = copy
  keywords + Surprise), `rename_mood(id, name)`, `delete_mood(id)` (not the last; deleting the active one activates the
  next), `set_active_mood(id)`, `move_mood(id, position)`; the existing keyword methods and `settings().surprise` act on
  the **active** mood (so hosts keep working); `history` gains a mood filter; `Generation` gains `mood_id`/`mood_name`.
  Typed InvalidInput reasons for mood names (empty, too long, duplicate) and `LastMood`.
- Behaviour: switching never generates by itself (no surprise spend). Novelty (memory) and taste stay **global**.
  Echo selection prefers originals made under the active mood (weight ×3), falling back to any.
- **As built (core):** names trimmed/single-spaced, 1–40 characters, unique case-insensitively; the migration names
  the first mood from its first two keywords that aren't Avoids (an Avoid names what's left out), capitalised, just
  the first when both pass 40 characters, else "My mood", and files every existing wallpaper under it.
  Id-based keyword methods (`set_keyword_weight`, `rename_keyword`, `move_keyword`, `remove_keyword`) work on any
  mood's keyword within its own mood, so the Moods detail pane can edit a mood that isn't active; adding to one is
  `add_mood_keyword`, its Surprise `set_mood_surprise`. New moods start with no keywords and Surprise 0.35;
  `copy_from` copies keywords (new ids) and Surprise. 64 keywords per mood. `keywords_are_narrow` looks at the active
  mood's last 5 new wallpapers. `update_settings` writes Surprise to the active mood: hosts read-modify-write
  promptly (a Settings copy held across a switch would carry the old mood's Surprise over).
- Hosts: macOS menu bar menu **Mood ▸** submenu (checkmarked, plus "Edit Moods…"); the main window's **Moods** section
  replaces Keywords: sidebar › name-only mood list (+ in the list header, right-click Use/Duplicate/Rename/Delete) ›
  detail (editable name above Surprise/stats, native form with keywords/Surprise/recent wallpapers/Delete Mood…) —
  user's layout 2026-10-06, simplified 2026-10-07; trailing Make/Stop toolbar action, see app-spec; App Intent "Switch Mood" (AppEnum of moods; Siri "Switch AutoPaper to
  Rainy beach"). Windows: Moods group in the NavigationView (each mood under it) + tray menu Mood submenu. GNOME: the sidebar is the mood
  list (AdwNavigationSplitView; Moods summary, then each mood; no separate list column, no per-row Use button: Use is in the mood's header,
  its context menu and the summary cards) and a KDE tray submenu. History filter by mood. All with spoken names/states ("Rainy beach, current mood").

## Retries that say what was wrong; `KeywordNotFollowed` (2026-10-06)
`compose` records every candidate's `Problem`s for the job (`Job.problems`); each retry's `ComposeContext.corrections` lists the most common
ones in the writer's own terms (`composer::correction`: "left out the Must keyword “x”: write its own words in the prompt", etc.). When every
idea fails and the most common problem is a Must left out or an Avoid used, `no_usable` returns `AutoPaperError::KeywordNotFollowed
{ keyword, weight, mood_id }` (FFI-exported); otherwise `InvalidResponse` with the problem in its English `detail`. Hosts show one line that names the keyword
and link to that mood (macOS `KeyProblem`, Windows `Problem.MoodId`, Linux `Fix::Mood`). Google painting uses `/v1beta/models/{model}:generateContent`
(the list's API version; preview models and Nano Banana 2.1 are not on `/v1`).

## Updates architecture (2026-10-07)
No AutoPaper server. macOS: Sparkle checks `https://msitarzewski.github.io/AutoPaper/appcast.xml` (EdDSA-signed zip on GitHub Releases, no system profile).
Windows: App Installer checks `https://msitarzewski.com/app-updates/autopaper/AutoPaper.appinstaller` on launch and about every 8 hours and installs silently
by the next launch (needs the Microsoft-chain signature); winget (when listed) uses the GitHub Release bundle. Linux: Flatpak/GNOME Software/Discover
use the signed repository `https://msitarzewski.com/app-updates/autopaper/flatpak` (sends `Flatpak-Ref`/`Flatpak-Upgrade-From`, not identifiers); the GNOME
runtime still comes from Flathub's runtime repository. Windows capabilities: `runFullTrust`, `picturesLibrary` (read the lock screen picture only),
`unvirtualizedResources` (registry write virtualization off so the lock screen change takes). See `techContext.md` for hosting and signing.

## Accessibility (all platforms)
WCAG 2.2 AA via WCAG2ICT. Every control/image named; each keyword speaks its state ("rain, Must");
Surprise speaks its value and label; the wallpaper's description is `summary` (+ echo note); rating state
is announced; no colour-only meaning; no motion beyond cross-fades; generation can be paused (2.2.2);
system Reduce Motion / high contrast / Reduce Transparency honoured; stock controls over custom ones.
Audited per platform through its accessibility API (AX, UI Automation, AT-SPI) plus a screen-reader pass.

## Testing seams
`HttpClient` trait (`testing::StubHttp`; recorded or documented-shape fixtures, no live calls in tests; WebSockets
through `HttpClient::open_socket`, scripted with `StubHttp::socket` — text, waits on tokio's clock, close, break),
`SecretStore` stub, `Clock` (`testing::FixedClock`, to simulate years of memory/echoes), `Embedder` trait
(`HashingEmbedder` + the real model in an ignored-by-default calibration test), the Demo provider (concepts
from `ComposeInputs`, gradient images, no cost). Live checks against the local ComfyUI, ignored by default:
`cargo test -p autopaper-core --lib comfyui::tests::live -- --ignored --nocapture` (a 512×288 painting, a
cancel that must leave ComfyUI idle with the job interrupted, and since round 4 at 768×432: every sampler step heard
on /ws, a stall found and the job stopped — the socket muted after step 3, step window shortened to 3 s — and a job
stopped in ComfyUI reported `Stopped` within 2 s; they skip when ComfyUI is busy). Demo's slow mode
(`AUTOPAPER_DEMO_DELAY_SECS`) drives progress UI in VMs. `core/tests/engine.rs` drives the whole engine (OpenAI over
StubHttp for retries and failures; Demo + FixedClock + HashingEmbedder for multi-year runs).
CLI `--display WxH` passes a display hint like a host (e.g. `autopaper --display 4112x2658 generate`).
`autopaper models [images|concepts]` prints what a host's model picker gets from `list_models` (id, name).
CLI `autopaper simulate --days N [--seed S] [--real-model]` runs the whole agent over simulated years with
fakes and reports novelty, echoes and taste. Dev and test builds optimise dependencies
(`[profile.dev.package."*"] opt-level = 2`); the core itself stays debug.
