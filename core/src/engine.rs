//! The engine: the one object every host holds. All agent decisions happen behind these methods.
//!
//! Threading: methods are safe to call from any thread. Sync methods do their work (SQLite, files, and for
//! some image decoding and resizing) on the caller's thread, so hosts call them off their UI thread. Async
//! methods run on whichever thread polls them (UniFFI polls on the host's executor; Rust hosts poll them
//! inside a tokio runtime): image decoding, thumbnails and embeddings go to the runtime's blocking pool,
//! SQLite work runs inline, and `ProgressObserver` is called on the polling thread. Only one generation runs
//! at a time; a second `generate` while one is running waits for it.
//!
//! How a wallpaper is made (`generate`, `run_if_due`, `make_echo`):
//! 1. Settings, keywords and providers; a budget check against the estimated cost (paid providers only).
//! 2. Compose: ask the text model for `composer::CANDIDATES` candidates, parse and check them (Musts, Avoids,
//!    fields), embed each, assess novelty against all of memory with the calibration of the embedding model
//!    in use (an echo is assessed outside its own lineage and must sit in the echo band of its original and
//!    visibly change at least one requested axis), score with taste and seeded jitter. When no valid
//!    candidate is novel, ask again (at most twice) naming the remembered wallpapers they were too close to,
//!    but stop as soon as a retry doesn't lower the least similar valid candidate's penalty (the keywords
//!    leave no room; asking again only costs); then take the least similar valid one and mark the
//!    generation `least_similar` (`keywords_are_narrow` counts them). A scheduled echo that yields no usable
//!    echo becomes an ordinary new wallpaper.
//! 3. Paint at the provider size closest to the primary display's aspect (largest available, but never
//!    more pixels than the display for free-size providers). The engine doesn't enumerate displays: it aims
//!    at `DEFAULT_DISPLAY` (3840×2160, 16:9) until the host calls `set_display_hint`.
//! 4. Decode with limits; write the original (`images/<yyyy>/<id>.<ext>`), a thumbnail (`thumbs/<id>.jpg`)
//!    and a perceptual hash; price it (the provider's reported cost, else `pricing`); store the generation
//!    and its embedding; prune to the storage limit (never the new wallpaper: the host hasn't shown it yet).
//!    Every new wallpaper, whatever asked for it, restarts the schedule (`next_due` counts from it).
//!
//! A refusal (content policy) gets one recompose asking for a gentler scene, then fails. Failures store a
//! failed or refused row (no embedding, redacted error) and return the error. Each paid call's cost goes into
//! the month's spend as soon as it answers, so failed, cancelled and dropped attempts count too. Progress is
//! reported at each stage when an observer is given (an observer that fails is logged and ignored).
//!
//! `cancel` takes effect at once: a provider request under way is dropped (its connection closed; a ComfyUI job
//! is stopped on the server), and requests waiting for their turn stop waiting.
//!
//! **Moods (2026-10-06):** a mood is a name, its keywords and its Surprise (`Mood`); one is active, and the keyword
//! methods without a mood and `Settings::surprise` act on it, so hosts written before moods keep working. A
//! wallpaper notes the mood it was made under (`Generation::mood_id`/`mood_name`); echoes prefer originals made
//! under the active mood (×3); novelty, taste and everything else in Settings are shared. Switching moods never
//! makes a wallpaper by itself.
//!
//! **Performance history (2026-10-06):** every finished provider call is timed (`perf::Timing`) and stored, so
//! `estimate` can say how long a painter or writer takes on this computer; a painting reports a fraction and
//! seconds left to the `ProgressDetailObserver` (from the painter's steps, else the estimate); ComfyUI gives up on
//! a job only when it stalls or runs past max(30 minutes, 3 × its estimate); and a scheduled wallpaper may start
//! early by its estimated duration (`next_start`) so it is ready when due. Demo has a slow mode for testing
//! progress UI (`AUTOPAPER_DEMO_DELAY_SECS`, read by `open`).
//!
//! The wallpaper on the desktop is the one most recently marked shown (`mark_shown`). Its display renders are
//! never deleted while it's there — not by `prune`, `delete_generation` or `clear_history` — because the OS
//! shows them by path; once another wallpaper is shown, they are deleted with the next prune if no generation
//! in history owns them any more.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use image::GenericImageView;
use rand::rngs::StdRng;
use rand::{Rng, RngCore, SeedableRng};
use serde_json::Value;
use uuid::Uuid;

use crate::composer::{self, ComposeContext, Composed, EchoBrief, Problem};
use crate::echo::{self, EchoAxis};
use crate::embed::{self, CandleEmbedder, HashingEmbedder};
use crate::error::{AutoPaperError, InvalidInputReason, ProviderUnavailableReason, Result};
use crate::imaging::{self, DecodeLimits};
use crate::model::*;
use crate::net::{self, HostPolicy, ReqwestClient};
use crate::novelty::{self, Calibration, NoveltyPolicy, NoveltyReport};
use crate::perf::{self, PaintTracker, Timing};
use crate::ports::{Clock, Embedder, HttpClient, HttpMethod, HttpRequest, HttpResponse, Socket, ProgressDetailObserver, ProgressObserver, SecretStore, SystemClock};
use crate::pricing;
use crate::providers::demo::{self, Demo};
use crate::providers::registry::{self, ProviderDeps};
use crate::providers::{ImageProvider, ImageRequest, ImageResponse, PaintProgress, PaintStep, TextProvider, google};
use crate::schedule;
use crate::store::{MemoryRow, Store, StoredGeneration};
use crate::taste::{self, Taste};
use crate::text::feature_key;

#[derive(Debug, Clone, uniffi::Record)]
pub struct EngineConfig {
    /// Where the database, images, thumbnails and renders live. Created if missing.
    pub data_dir: String,
    /// Directory holding the sentence-embedding model files (shipped in the app bundle/package).
    pub model_dir: String,
    /// BCP 47, e.g. "en-US"; titles, summaries and echo notes are written in this language.
    pub locale: String,
    /// Appended to the User-Agent, e.g. "macOS/26.1 AutoPaper/0.1.0".
    pub client: String,
}

/// Why a past wallpaper was brought back instead of a new one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum RevisitReason {
    Requested,
    OverBudget,
    Offline,
    ProviderFailed,
    ServicesUnavailable,
}

/// What to put on the desktop.
#[derive(Debug, Clone, uniffi::Record)]
pub struct Shown {
    pub generation: Generation,
    /// Set when this is a past wallpaper brought back rather than a new one.
    pub revisit: Option<RevisitReason>,
}

/// Which sentence-embedding model memory uses (Settings → Memory; smoke tests assert the real model).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MemoryStatus {
    /// The model's id, e.g. "BAAI/bge-small-en-v1.5".
    pub embedding_model: String,
    /// Memory runs on the hashing fallback, which only catches near-verbatim repeats: hosts say so
    /// ("Memory is running in reduced mode").
    pub reduced: bool,
    /// Why the real model couldn't be loaded, when `open` tried and failed (for logs and support; English).
    pub problem: Option<String>,
}

/// The display size the engine paints for until a host passes a hint: 4K at 16:9.
pub const DEFAULT_DISPLAY: (u32, u32) = (3840, 2160);

/// Demo's slow mode for testing progress UI (seconds a Demo wallpaper takes, 1–3600), read by `Engine::open`.
pub const DEMO_DELAY_VARIABLE: &str = "AUTOPAPER_DEMO_DELAY_SECS";
/// How often a painting's progress detail is looked at (it's reported only when it changed).
const DETAIL_TICK: Duration = Duration::from_secs(1);

const DATABASE_FILE: &str = "autopaper.sqlite3";
/// Compose calls repeated (after the first) when no valid candidate is novel enough.
const MAX_COMPOSE_RETRIES: usize = 2;
/// Most "too close" summaries carried into a retry (the composer quotes at most 8).
const MAX_TOO_CLOSE: usize = 8;
const MIN_STORAGE_LIMIT_MB: u32 = 256;
/// Largest display side `render_for_display` accepts (imaging's decode limit).
const MAX_RENDER_SIDE: u32 = 16_384;
/// A new image within this many pHash bits of a recent one is logged as a near-duplicate.
const NEAR_DUPLICATE_BITS: u32 = 6;
const RECENT_IMAGES_COMPARED: u32 = 30;
/// Most recent liked and most recent disliked wallpapers read to put learned taste into words.
const TASTE_LABEL_SOURCES: u32 = 500;
/// An image file no generation owns is deleted only once it is this old (by its modification time): another
/// process using the same data directory may be between writing a new wallpaper's files and storing its row.
const ORPHAN_MIN_AGE: std::time::Duration = std::time::Duration::from_secs(60 * 60);
/// Failed and refused attempts are kept this long (the CLI and support read them), then `prune` deletes them.
const FAILED_ROWS_KEPT_DAYS: i64 = 30;
const SECONDS_PER_DAY: i64 = 86_400;
/// `keywords_are_narrow`: at least `NARROW_FALLBACKS` of the last `NARROW_WINDOW` new wallpapers (not
/// echoes) took the least similar candidate.
const NARROW_WINDOW: u32 = 5;
const NARROW_FALLBACKS: usize = 3;

// Engine state (the store's key-value table).
/// Scheduled attempts that failed in a row.
const STATE_FAILURES: &str = "consecutive_failures";
/// No scheduled attempt before this Unix time (transient-failure backoff).
const STATE_BACKOFF_UNTIL: &str = "backoff_until";
/// When the schedule last restarted: the newest new wallpaper, whatever asked for it (scheduled, "New
/// Wallpaper Now", a disliked one's replacement, an echo; revisits don't count), or a slot skipped over budget.
/// It counts from here. Clamped to now when the clock is set back past it.
const STATE_LAST_SLOT_AT: &str = "last_slot_at";
/// How many seconds before it was due the run that filled the last slot started (`next_start`): the next one is
/// one interval after the due time. 0 (or missing) otherwise.
const STATE_SLOT_LEAD: &str = "slot_lead";
/// Read only from databases written before `last_slot_at`: when a slot was skipped over budget.
const STATE_SLOT_SKIPPED_AT: &str = "slot_skipped_at";
/// The id of the generation most recently marked shown: the one whose renders are on the desktop, even after
/// it was cleared from history or deleted. A database without it uses `current()`.
const STATE_DESKTOP: &str = "desktop";

/// What `Engine::open` builds for itself; tests and the CLI pass their own to `Engine::open_with`
/// (Rust only, not exported).
pub struct Deps {
    pub clock: Arc<dyn Clock>,
    pub http: Arc<dyn HttpClient>,
    pub embedder: Arc<dyn Embedder>,
    /// Seeds every random choice the engine makes (echo chance and pick, echo axes, score jitter, Demo
    /// provider seeds, image seeds), so a run with fake providers and a fixed clock is reproducible.
    pub rng_seed: u64,
}

/// Counters since the engine opened (Rust only: the CLI's `simulate` and tests read them).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EngineStats {
    pub compose_calls: u32,
    /// Valid candidates passed over because they were too similar to memory.
    pub too_similar_candidates: u32,
    /// Compose calls repeated because valid candidates were all too similar to memory.
    pub novelty_retries: u32,
    /// Compose calls repeated because no candidate passed the checks (keywords, echo band, unreadable).
    pub invalid_retries: u32,
    /// Retries not made because the last one didn't lower the least similar valid candidate's penalty.
    pub retries_stopped: u32,
    /// Wallpapers that took the least similar valid candidate after the retries.
    pub least_similar_fallbacks: u32,
    /// Recomposes after a refusal.
    pub refusal_recomposes: u32,
    /// Remembered generations re-embedded after an embedding-model change.
    pub reembedded: u32,
}

/// One candidate of a dry-run compose (`Engine::compose_preview`).
#[derive(Debug, Clone)]
pub struct CandidatePreview {
    pub concept: Concept,
    pub echo_note: Option<String>,
    /// Failed checks, in plain words; empty when it passes.
    pub problems: Vec<String>,
    /// Highest age-weighted similarity to anything remembered (0 with an empty memory).
    pub similarity: f32,
    /// Title of the closest remembered wallpaper.
    pub nearest: Option<String>,
    pub novel: bool,
    /// Learned taste for its features, about −0.5..0.5.
    pub taste: f32,
    pub score: f32,
    pub valid: bool,
}

/// A dry-run compose: one call to the text model, every candidate assessed, nothing painted or stored
/// (the text call's cost is still added to the month's spend).
#[derive(Debug, Clone)]
pub struct ComposePreview {
    pub candidates: Vec<CandidatePreview>,
    /// The candidate `generate` would take from this call (best valid novel one, else least similar valid).
    pub chosen: Option<usize>,
    pub threshold: f32,
    pub embedding_model: String,
    pub text_model: String,
    pub cost_microusd: u64,
}

/// AutoPaper's agent: one per data directory, held by the host for the app's lifetime.
///
/// Threading: call it from any thread. Sync methods block the calling thread for their database and file
/// work (the ones marked **Blocking** for longer: hundreds of milliseconds and more), so hosts call them off
/// their UI thread. Async methods do their database work on the thread that polls them and their image and
/// embedding work on the core's blocking pool; `ProgressObserver` is called on the polling thread, which is
/// not necessarily the UI thread, so hosts marshal. Rust hosts (the GTK app, the CLI) must poll the async
/// methods inside a tokio runtime context.
#[derive(uniffi::Object)]
pub struct Engine {
    inner: Arc<Inner>,
}

struct Inner {
    config: EngineConfig,
    dirs: Dirs,
    store: Arc<Mutex<Store>>,
    console: Arc<RunCapture>,
    secrets: Arc<dyn SecretStore>,
    clock: Arc<dyn Clock>,
    http: Arc<dyn HttpClient>,
    embedder: Arc<dyn Embedder>,
    rng: Mutex<StdRng>,
    /// Single flight: held for a whole generation.
    making: tokio::sync::Mutex<()>,
    /// `cancel` calls so far, watched so that a provider request under way, or a request waiting for its turn,
    /// stops the moment one comes. Each request notes the count before it waits for its turn
    /// (`cancel_ticket`); a later cancel stops it, whether it is running or still waiting.
    cancels: tokio::sync::watch::Sender<u64>,
    /// The ticket of the generation running now (`begin`), checked between stages.
    serving: AtomicU64,
    /// Why `open` couldn't load the real embedding model (memory then uses the hashing fallback).
    embedder_problem: Option<String>,
    /// Memory has been re-embedded with the current model (checked lazily before the first novelty check).
    memory_current: AtomicBool,
    display: Mutex<(u32, u32)>,
    stats: Mutex<EngineStats>,
    /// The host's `ProgressDetailObserver` (`set_progress_detail_observer`).
    detail: Mutex<Option<Arc<dyn ProgressDetailObserver>>>,
    /// Demo's slow mode, in milliseconds (0: off).
    demo_delay_ms: AtomicU64,
    /// The computer's appearance, as the host last reported it (`set_system_appearance`).
    appearance: Mutex<Option<Appearance>>,
}

#[derive(Debug, Clone)]
struct Dirs {
    root: PathBuf,
    images: PathBuf,
    thumbs: PathBuf,
    renders: PathBuf,
}

#[uniffi::export(async_runtime = "tokio")]
impl Engine {
    /// Opens (creating) the data directory and database. The embedding model comes from `model_dir`; if
    /// its files are missing the engine falls back to `HashingEmbedder` (logged, and reported by
    /// `memory_status`), which only catches near-verbatim repeats. Memory embedded with another model is
    /// re-embedded lazily, before the next novelty check. **Blocking**: loads the model (133 MB) and migrates
    /// the database.
    #[uniffi::constructor]
    pub fn open(config: EngineConfig, secrets: Arc<dyn SecretStore>) -> Result<Arc<Self>> {
        let (embedder, problem) = load_embedder(Path::new(&config.model_dir));
        let client = match config.client.trim() {
            "" => "autopaper-core",
            client => client,
        };
        let http: Arc<dyn HttpClient> = Arc::new(ReqwestClient::new(client)?);
        let deps = Deps { clock: Arc::new(SystemClock), http, embedder, rng_seed: rand::random() };
        let engine = Self::open_inner(config, secrets, deps, problem)?;
        if let Some(delay) = demo_delay_from_env() {
            tracing::info!(seconds = delay.as_secs(), "Demo's slow mode is on ({DEMO_DELAY_VARIABLE})");
            engine.set_demo_delay(delay);
        }
        Ok(engine)
    }

    /// Local Console runs, newest first. At most 100 per page, 200 retained for 30 days.
    pub fn runs(&self, limit: u32, offset: u32) -> Result<Vec<RunRecord>> {
        self.inner.store().prune_runs(self.inner.now())?;
        self.inner.store().runs(limit, offset)
    }

    pub fn run(&self, id: String) -> Result<RunRecord> {
        self.inner.store().run(&id)?.ok_or(AutoPaperError::NotFound)
    }

    /// Export only the already sanitized local Console record.
    pub fn run_report(&self, id: String) -> Result<String> {
        serde_json::to_string_pretty(&self.run(id)?).map_err(|error| internal(error.to_string()))
    }

    pub fn clear_runs(&self) -> Result<()> {
        let _idle = self.inner.making.try_lock().map_err(|_| invalid(InvalidInputReason::Other, "Stop the run before clearing the Console."))?;
        self.inner.store().clear_runs()
    }

    pub fn budget_status(&self) -> Result<BudgetStatus> {
        self.inner.budget_status()
    }

    /// Real outcomes and provider-call timings for retained Console runs, independent of list pagination.
    pub fn console_statistics(&self) -> Result<ConsoleStatistics> {
        let mut store = self.inner.store();
        store.prune_runs(self.inner.now())?;
        store.console_statistics()
    }

    // ── Moods ───────────────────────────────────────────────────────────────────────────────

    /// Every mood in the person's order, each with its keywords and Surprise; the active one has `active`.
    pub fn moods(&self) -> Result<Vec<Mood>> {
        self.inner.store().moods()
    }

    /// The mood in use: its keywords are what `keywords()` lists and its Surprise is `settings().surprise`.
    pub fn active_mood(&self) -> Result<Mood> {
        self.inner.store().active_mood()
    }

    /// A new mood at the end of the list (not made active). `copy_from` duplicates that mood's keywords and
    /// Surprise; without it the mood starts with no keywords and the default Surprise (0.35). `InvalidInput`
    /// `MoodNameEmpty`, `MoodNameTooLong` (over 40 characters) or `DuplicateMoodName` (case-insensitive);
    /// `NotFound` for an unknown `copy_from`.
    pub fn create_mood(&self, name: String, copy_from: Option<String>) -> Result<Mood> {
        let now = self.inner.now();
        self.inner.store().create_mood(&name, copy_from.as_deref(), now)
    }

    /// Renames a mood (errors as for `create_mood`; changing only its case is fine). `NotFound` for an unknown id.
    pub fn rename_mood(&self, id: String, name: String) -> Result<Mood> {
        self.inner.store().rename_mood(&id, &name)
    }

    /// Deletes a mood and its keywords; its wallpapers stay in History (with no mood name). Deleting the active
    /// mood makes the next one in the list active (the previous one when it was last). `InvalidInput` `LastMood`
    /// for the only mood; `NotFound` for an unknown id.
    pub fn delete_mood(&self, id: String) -> Result<()> {
        self.inner.store().delete_mood(&id)
    }

    /// Makes a mood active. Nothing is made by itself (no surprise spend): the next wallpaper, scheduled or asked
    /// for, uses its keywords and Surprise. `NotFound` for an unknown id.
    pub fn set_active_mood(&self, id: String) -> Result<()> {
        self.inner.store().set_active_mood(&id)
    }

    /// Moves a mood to `to_position` in the list (clamped), renumbering the others. `NotFound` for an unknown id.
    pub fn move_mood(&self, id: String, to_position: u32) -> Result<()> {
        self.inner.store().move_mood(&id, to_position)
    }

    /// Adds a keyword to any mood (the Moods detail pane edits moods that aren't active); as `add_keyword`
    /// otherwise. `NotFound` for an unknown mood.
    pub fn add_mood_keyword(&self, mood_id: String, text: String, weight: KeywordWeight) -> Result<Keyword> {
        let now = self.inner.now();
        self.inner.store().upsert_keyword_in(&mood_id, &text, weight, now)
    }

    /// Sets any mood's Surprise (clamped to 0–1; not a number → the default). For the active mood this is
    /// `settings().surprise`. `NotFound` for an unknown mood.
    pub fn set_mood_surprise(&self, mood_id: String, surprise: f32) -> Result<()> {
        self.inner.store().set_mood_surprise(&mood_id, clamped_surprise(surprise))
    }

    /// What every mood has made, one entry per mood in the person's order (as `moods()`; a mood that has made
    /// nothing is listed at zero): how many wallpapers History lists for it, how many are liked and disliked, when
    /// the newest was made, and its newest few (`MoodStats::LATEST`, newest first). For a summary of all moods
    /// without paging History per mood. **Blocking** for a large library (two queries).
    pub fn mood_stats(&self) -> Result<Vec<MoodStats>> {
        self.inner.store().mood_stats(MoodStats::LATEST as u32)
    }

    /// Wallpapers made per day and mood, for a chart of recent activity. `day_bounds` are ascending Unix times:
    /// day `i` runs from `day_bounds[i]` up to `day_bounds[i + 1]`, so n + 1 values give n days (pass the host's
    /// local midnights, which keeps its time zone and daylight saving right). Counts what History lists (as
    /// `mood_stats`). Only days and moods with wallpapers are listed, by day, then in the moods' order;
    /// wallpapers of a deleted mood come last on their day, with `mood_id` `None`. `InvalidInput` (`Other`) for
    /// fewer than 2 or more than `DayCount::MAX_DAYS` + 1 bounds, or bounds that don't ascend.
    pub fn activity(&self, day_bounds: Vec<i64>) -> Result<Vec<DayCount>> {
        self.inner.store().activity(&day_bounds)
    }

    // ── Keywords ────────────────────────────────────────────────────────────────────────────

    /// The active mood's keywords, in order.
    pub fn keywords(&self) -> Result<Vec<Keyword>> {
        self.inner.store().keywords()
    }

    /// Adds a keyword to the active mood (trimmed, single-spaced, ≤ 40 chars). A duplicate in that mood
    /// (case-insensitive) returns the existing keyword with its weight updated.
    pub fn add_keyword(&self, text: String, weight: KeywordWeight) -> Result<Keyword> {
        let now = self.inner.now();
        self.inner.store().upsert_keyword(&text, weight, now)
    }

    /// Any mood's keyword, by id.
    pub fn set_keyword_weight(&self, id: String, weight: KeywordWeight) -> Result<()> {
        self.inner.store().set_keyword_weight(&id, weight)
    }

    /// Any mood's keyword, by id; `DuplicateKeyword` when another keyword of its mood has the text.
    pub fn rename_keyword(&self, id: String, text: String) -> Result<Keyword> {
        self.inner.store().rename_keyword(&id, &text)
    }

    /// Any mood's keyword, by id, within its mood.
    pub fn move_keyword(&self, id: String, to_position: u32) -> Result<()> {
        self.inner.store().move_keyword(&id, to_position)
    }

    /// Any mood's keyword, by id.
    pub fn remove_keyword(&self, id: String) -> Result<()> {
        self.inner.store().delete_keyword(&id)
    }

    /// True when the active mood's keywords leave too little room for new ideas: at least 3 of the last 5 new
    /// wallpapers made under it (successful, not echoes, any trigger; cleared history counts, since memory keeps
    /// it) found no candidate novel enough even after asking again, and took the least similar one. Hosts show a
    /// gentle note on the Keywords view ("Your keywords are narrow, so new ideas are getting hard to find. Add
    /// some Maybes or raise Surprise for more variety."). It turns false again as new wallpapers find novel ideas.
    pub fn keywords_are_narrow(&self) -> Result<bool> {
        let recent = {
            let store = self.inner.store();
            let mood = store.active_mood_id()?;
            store.recent_least_similar(NARROW_WINDOW, Some(&mood))?
        };
        Ok(recent.iter().filter(|&&least_similar| least_similar).count() >= NARROW_FALLBACKS)
    }

    // ── Settings ────────────────────────────────────────────────────────────────────────────

    pub fn settings(&self) -> Result<Settings> {
        self.inner.store().settings()
    }

    /// Validates and saves; `surprise` goes to the active mood (read-modify-write promptly: a Settings copy held
    /// across a mood switch would give the new mood the old one's Surprise). Out-of-range values are clamped
    /// (surprise 0–1, a non-number → the default; storage ≥ 256 MB). Model names and base URLs are trimmed (a blank URL means the default); a base URL
    /// the network policy refuses is `InvalidInput`, and a provider that can't do its job (ComfyUI for
    /// concepts, Ollama for images) is `Unsupported`. A blank ComfyUI workflow means the bundled one.
    pub fn update_settings(&self, settings: Settings) -> Result<()> {
        let settings = validated(settings)?;
        self.inner.store().save_settings(&settings)
    }

    // ── Making wallpapers ───────────────────────────────────────────────────────────────────

    /// Composes, checks memory, generates and stores a new wallpaper. Does not set it: the host
    /// renders it for each display, sets it, then calls `mark_shown`. `Trigger::EchoRequest` echoes an
    /// original the agent picks (one past its quiet period if any, else any); `make_echo` names one.
    /// `BudgetReached` when a paid provider's estimated cost would pass the monthly cap.
    pub async fn generate(&self, trigger: Trigger, observer: Option<Arc<dyn ProgressObserver>>) -> Result<Generation> {
        self.make_request(trigger, None, observer, false).await.map(|shown| shown.generation)
    }

    /// Makes a wallpaper, or returns the newest usable saved image from the selected mood when
    /// either selected service fails its availability check. A revisit is not a new generation.
    pub async fn generate_or_revisit(&self, trigger: Trigger, observer: Option<Arc<dyn ProgressObserver>>) -> Result<Shown> {
        self.make_request(trigger, None, observer, true).await
    }

    /// Called on the host's timer and on wake. Makes a wallpaper if one is due (an echo, by chance,
    /// when eligible). If a service fails its preflight check, returns the newest usable saved image from
    /// the selected mood, independent of fallback preference. Later failures, with fallback
    /// `RevisitLiked`, return a liked past wallpaper instead. `None` when nothing is due or paused.
    ///
    /// Over budget, the slot counts as filled (the next try is one interval later), `BudgetReached` is
    /// returned and the current wallpaper stays in place regardless of the fallback preference. Any
    /// failure backs off (10 min, doubling, at most 1 h; see `next_due`). The first failure in a row of a
    /// transient kind, a refusal or an unusable answer revisits a liked wallpaper (`Offline` or
    /// `ProviderFailed`); later ones, and failures the person has to fix (a missing or rejected key, a bad
    /// address), return the error so the host can say what's wrong. A painting ComfyUI couldn't run
    /// (`needs_setup`: `InvalidResponse` or `InvalidInput` from ComfyUI) is never covered by a revisit and fills
    /// its slot: trying again on the timer can't fix it. A rate limit waits at least as long as
    /// the provider's `Retry-After`. A cancelled run (`Cancelled`, also when it was cancelled while waiting for
    /// its turn) isn't a failure — no backoff — but it fills its slot, like one skipped over budget: the next
    /// scheduled wallpaper is one interval later, so a host's timer doesn't start it again at once.
    ///
    /// It counts as due from `next_start` (the due time less the wallpaper's estimated duration), so a wallpaper
    /// started early is ready on time; it fills the slot it was due for (the next is one interval after the due
    /// time, not after its early start). Hosts that wake at `next_due` get the old behaviour.
    pub async fn run_if_due(&self, observer: Option<Arc<dyn ProgressObserver>>) -> Result<Option<Shown>> {
        let inner = &self.inner;
        let ticket = inner.cancel_ticket();
        let making = inner.turn(ticket).await;
        let now = inner.now();
        let settings = inner.store().settings()?;
        let Some((due, start)) = inner.schedule_at(&settings, now)? else { return Ok(None) };
        if !schedule::is_due(Some(start), now) {
            return Ok(None);
        }
        let result = match making {
            Ok(_making) => {
                inner.start_run(Trigger::Scheduled)?;
                let _run = RunGuard::new(inner.console.clone());
                let result = inner.run_due(ticket, &settings, now, due, observer.as_ref()).await;
                inner.finish_run(result.as_ref().map(|shown| shown.as_ref().map(|shown| &shown.generation)))?;
                result
            }
            Err(cancelled) => Err(cancelled),
        };
        if matches!(result, Err(AutoPaperError::Cancelled)) {
            inner.fill_slot(now, due)?;
        }
        result
    }

    /// Unix seconds when the next scheduled wallpaper is due; `None` when paused or manual. Counts from the
    /// newest new wallpaper of any trigger ("New Wallpaper Now", a disliked one's replacement and echoes
    /// restart the schedule; revisits don't), or from a slot skipped over budget, and never before a failure's
    /// backoff ends.
    pub fn next_due(&self) -> Result<Option<i64>> {
        let now = self.inner.now();
        let settings = self.inner.store().settings()?;
        self.inner.next_due_at(&settings, now)
    }

    /// Unix seconds when the next scheduled wallpaper should start so it's ready by `next_due`: the due time less
    /// how long a wallpaper takes here (the writing and painting estimates for the current providers and display;
    /// at most half an interval, and never before a failure's backoff ends). Equal to `next_due` while nothing is
    /// recorded. `None` when paused or manual. Hosts set their timer for this (`run_if_due` is due from then on).
    pub fn next_start(&self) -> Result<Option<i64>> {
        let now = self.inner.now();
        let settings = self.inner.store().settings()?;
        Ok(self.inner.schedule_at(&settings, now)?.map(|(_, start)| start))
    }

    /// Makes an echo of a past wallpaper now (ignores the quiet period). `NotFound` for an unknown id;
    /// `InvalidInput` for a generation that failed.
    pub async fn make_echo(&self, id: String, observer: Option<Arc<dyn ProgressObserver>>) -> Result<Generation> {
        self.make_request(Trigger::EchoRequest, Some(id), observer, false).await.map(|shown| shown.generation)
    }

    /// Makes an explicit echo, with the same selected-mood availability fallback as generate_or_revisit.
    pub async fn make_echo_or_revisit(&self, id: String, observer: Option<Arc<dyn ProgressObserver>>) -> Result<Shown> {
        self.make_request(Trigger::EchoRequest, Some(id), observer, true).await
    }

    /// A liked past wallpaper, least recently shown first (not the one showing, when there's another).
    /// Costs nothing. `NothingToRevisit` when no liked wallpaper still has its image.
    pub fn revisit_liked(&self) -> Result<Shown> {
        self.inner.revisit(RevisitReason::Requested)
    }

    /// Stops the generation in progress, and any already waiting for their turn: they return `Cancelled`
    /// promptly. A provider request under way is dropped (its connection closed; a ComfyUI job is stopped on the
    /// server); what a provider had already reported as spent stays in the month's spend. A paid request
    /// dropped mid-way may still be billed by the provider; its cost isn't known, so it isn't counted.
    /// Generations asked for after this call aren't affected. A cancelled scheduled run fills its slot (see
    /// `run_if_due`).
    pub fn cancel(&self) {
        self.inner.cancels.send_modify(|count| *count = count.wrapping_add(1));
    }

    // ── What's showing, and history ─────────────────────────────────────────────────────────

    /// The host has put this generation on the desktop. Its display renders are kept from now until another
    /// wallpaper is marked shown, even if it is deleted or history is cleared (the OS shows them by path).
    pub fn mark_shown(&self, id: String) -> Result<()> {
        let now = self.inner.now();
        let mut store = self.inner.store();
        store.mark_shown(&id, now)?;
        store.state_set(STATE_DESKTOP, &Value::String(id))
    }

    /// The wallpaper most recently marked shown.
    pub fn current(&self) -> Result<Option<Generation>> {
        self.inner.store().current()
    }

    /// Newest first. **Blocking** for large pages.
    pub fn history(&self, filter: HistoryFilter, limit: u32, offset: u32) -> Result<Vec<Generation>> {
        self.inner.store().history(filter, limit, offset)
    }

    /// `history` of the wallpapers made under one mood (History's mood filter; a mood's recent wallpapers);
    /// `None` = every mood. An unknown or deleted mood's id lists what was made under it (nothing, for an id
    /// that never existed). **Blocking** for large pages.
    pub fn history_by_mood(
        &self,
        filter: HistoryFilter,
        mood_id: Option<String>,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<Generation>> {
        self.inner.store().history_by_mood(filter, mood_id.as_deref(), limit, offset)
    }

    pub fn generation(&self, id: String) -> Result<Generation> {
        self.inner.store().generation(&id)?.map(|stored| stored.generation).ok_or(AutoPaperError::NotFound)
    }

    /// The original (if this is an echo) and every echo in its lineage, oldest first.
    pub fn lineage(&self, id: String) -> Result<Vec<Generation>> {
        self.inner.store().lineage(&id)
    }

    /// True when `lineage(id)` would list more than this wallpaper: it is an echo of one still in History, or
    /// one in History echoes it (or echoes its original). Hosts offer "Show Original and Echoes" only then.
    /// One cheap query (no generations loaded), so hosts can ask per History item or when opening its menu.
    /// `NotFound` for an unknown id.
    pub fn has_echoes(&self, id: String) -> Result<bool> {
        self.inner.store().has_relatives(&id)
    }

    /// The wallpaper in words for screen readers and "What's on My Desktop": title, summary and echo note, as
    /// sentences, in the language they were written in (`EngineConfig.locale`). The rating isn't included:
    /// hosts say it in their own language (and the rating buttons announce their state).
    pub fn describe(&self, id: String) -> Result<String> {
        Ok(description(&self.generation(id)?))
    }

    /// A JPEG at the display's native size (centre-cropped, scaled); returns its path. Cached as
    /// `renders/<id>-<w>x<h>.jpg`. `InvalidInput` for a side outside 1–16384 (or over 64 MP); `NotFound`
    /// for an unknown id or a pruned image. **Blocking**: decodes and resizes a 4K-class image.
    pub fn render_for_display(&self, id: String, display: DisplayTarget) -> Result<String> {
        let (width, height) = (display.width, display.height);
        if !(1..=MAX_RENDER_SIDE).contains(&width) || !(1..=MAX_RENDER_SIDE).contains(&height) {
            return Err(invalid(InvalidInputReason::DisplaySizeInvalid, format!("can't render for a {width}×{height} display")));
        }
        let generation = self.generation(id)?;
        let source = generation.image_path.ok_or(AutoPaperError::NotFound)?;
        let target = self.inner.dirs.renders.join(format!("{}-{width}x{height}.jpg", generation.id));
        if target.is_file() {
            return Ok(path_string(&target));
        }
        let bytes = match fs::read(&source) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == ErrorKind::NotFound => return Err(AutoPaperError::NotFound),
            Err(error) => return Err(error.into()),
        };
        let (_, picture) = imaging::decode(&bytes, &DecodeLimits::default())?;
        imaging::render_cover(&picture, width, height, &target)?;
        Ok(path_string(&target))
    }

    // ── Learning ────────────────────────────────────────────────────────────────────────────

    /// Records a rating and updates taste. Returns true when the host should replace the wallpaper
    /// now (disliked, it's the one showing, and "Replace wallpapers I dislike" is on), and a new one fits
    /// the budget. **Blocking** (a database transaction).
    pub fn rate(&self, id: String, rating: Rating) -> Result<bool> {
        let inner = &self.inner;
        let now = inner.now();
        let (previous, showing, settings) = {
            let mut store = inner.store();
            let stored = store.generation(&id)?.ok_or(AutoPaperError::NotFound)?;
            let previous = store.set_rating(&id, rating, now)?;
            let mut learned = Taste::from_rows(store.taste_rows()?);
            let changed = learned.record(&taste::features(&stored.generation.concept), previous, rating, now);
            store.save_taste_rows(&changed)?;
            let showing = store.current()?.is_some_and(|current| current.id == id);
            (previous, showing, store.settings()?)
        };
        let replace =
            rating == Rating::Disliked && previous != Rating::Disliked && showing && settings.replace_disliked;
        Ok(replace && inner.budget_allows(&settings, now, inner.estimate_cost(&settings))?)
    }

    /// Learned features in the words the wallpapers used ("warm ivory", not the stem key).
    pub fn taste_summary(&self) -> Result<TasteSummary> {
        let now = self.inner.now();
        let summary = {
            let store = self.inner.store();
            Taste::from_rows(store.taste_rows()?).summary(now, store.rated_count()?)
        };
        let labels = self.inner.taste_labels()?;
        Ok(TasteSummary {
            liked: in_words(summary.liked, &labels),
            disliked: in_words(summary.disliked, &labels),
            ratings: summary.ratings,
        })
    }

    /// Forgets learned taste (ratings stay on the generations).
    pub fn reset_taste(&self) -> Result<()> {
        self.inner.store().reset_taste()
    }

    // ── Providers and cost ──────────────────────────────────────────────────────────────────

    /// Checks the key/address with a cheap call (listing models). Errors say what's wrong.
    pub async fn test_provider(&self, selection: ProviderSelection, job: ProviderJob) -> Result<()> {
        self.list_models(selection, job).await.map(|_| ())
    }

    pub async fn list_models(&self, selection: ProviderSelection, job: ProviderJob) -> Result<Vec<ModelInfo>> {
        let settings = self.inner.store().settings()?;
        match job {
            ProviderJob::Concepts => {
                let provider = self.inner.text_provider(&selection, &settings)?;
                provider.list_models().await
            }
            ProviderJob::Images => {
                let provider = self.inner.image_provider(&selection, &settings)?;
                provider.list_models().await
            }
        }
    }

    /// About how many seconds (rounded, at least 1) `job` takes with `selection` on this computer, from the calls
    /// recorded here: a
    /// recency-weighted median (recent ones count most; a month-old call counts half), scaled to `width` ×
    /// `height` by pixels for paintings — 0 × 0 means the size AutoPaper would ask this painter for now (the
    /// display hint, the model's limits) — and to the painter's steps. `None` until a call like it has finished
    /// here (never a made-up number). Settings can say "About 9 minutes per wallpaper on this Mac" (the writing
    /// and the painting estimates together). Recorded per server, so another computer's ComfyUI has its own.
    pub fn estimate(&self, selection: ProviderSelection, job: ProviderJob, width: u32, height: u32) -> Result<Option<u32>> {
        let settings = self.inner.store().settings()?;
        let seconds = match job {
            ProviderJob::Concepts => {
                let provider = self.inner.quiet_text_provider(&selection, &settings)?;
                let model = model_or(&selection.model, provider.default_model()).to_string();
                self.inner.estimate_secs(job, provider.kind(), &origin_of(&selection), &model, 0, 0, None)?
            }
            ProviderJob::Images => {
                let provider = self.inner.quiet_image_provider(&selection, &settings)?;
                let model = model_or(&selection.model, provider.default_model()).to_string();
                let (width, height) = match (width, height) {
                    (0, _) | (_, 0) => self.inner.request_size(provider.as_ref(), &model),
                    size => size,
                };
                let steps = provider.steps(&selection.model);
                self.inner.estimate_secs(job, provider.kind(), &origin_of(&selection), &model, width, height, steps)?
            }
        };
        Ok(seconds.map(perf::whole_seconds))
    }

    /// Registers the host's detail observer (`None` removes it): every generation then reports its stage, and while
    /// painting a fraction and seconds left (`ProgressDetail`), on top of the per-call `ProgressObserver`.
    pub fn set_progress_detail_observer(&self, observer: Option<Arc<dyn ProgressDetailObserver>>) {
        *lock(&self.inner.detail) = observer;
    }

    /// The computer's light or dark appearance (`None`: unknown). Hosts report it at launch and whenever it changes;
    /// while `Settings::match_system_theme` is on, new ideas are asked to suit it.
    pub fn set_system_appearance(&self, appearance: Option<Appearance>) {
        *lock(&self.inner.appearance) = appearance;
    }

    /// This month's (UTC) estimated spend, and estimates for one more wallpaper (a typical compose call
    /// plus one image at the size the current settings would request) and for a month at the cadence.
    pub fn spend_summary(&self) -> Result<SpendSummary> {
        let inner = &self.inner;
        let now = inner.now();
        let settings = inner.store().settings()?;
        let month = schedule::month_key(now);
        let (spent, images) = inner.store().spend(&month)?;
        let per_image = inner.estimate_cost(&settings);
        let monthly = (per_image as f64 * schedule::runs_per_month(settings.cadence)).round() as u64;
        Ok(SpendSummary {
            month,
            spent_microusd: spent,
            images,
            budget_cents: settings.monthly_budget_cents,
            per_image_microusd: per_image,
            monthly_estimate_microusd: monthly,
        })
    }

    // ── Storage ─────────────────────────────────────────────────────────────────────────────

    /// `image_bytes` counts originals, thumbnails and display renders; `generations` is what memory holds.
    /// **Blocking**: walks the image folders.
    pub fn storage_usage(&self) -> Result<StorageUsage> {
        self.inner.usage()
    }

    /// Enforces the storage limit: deletes image files of the oldest unliked wallpapers (never the
    /// current one or the one on the desktop, never liked ones), keeping their memory. Display renders (a
    /// cache) go first; thumbnails stay, so History still shows a picture. A file that can't be deleted
    /// (another app has it open) stays tracked and is tried again next time. Also runs after every new
    /// wallpaper, which it never removes. When no generation is being made, also deletes image files no
    /// generation owns (left by a call the host dropped mid-way) and display renders no wallpaper in history
    /// owns once they're off the desktop. Failed and refused attempts older than 30 days are deleted too (they
    /// hold no files). **Blocking**: walks and deletes files.
    pub fn prune(&self) -> Result<()> {
        // Whether a file is owned can only be told while no generation is between writing and storing it.
        if let Ok(_making) = self.inner.making.try_lock() {
            self.inner.remove_orphans()?;
        }
        self.inner.forget_old_failures()?;
        self.inner.prune_files(None).map(|_| ())
    }

    /// Deletes one generation and its files. Echoes of it keep their link (their lineage stays together).
    /// The wallpaper on the desktop keeps its display renders (the OS shows them by path) until another one is
    /// shown; its original and thumbnail go. `Storage` when a file couldn't be deleted (another app has it
    /// open): the generation stays, holding only the files that are left, so the call can be repeated.
    /// **Blocking**.
    pub fn delete_generation(&self, id: String) -> Result<()> {
        let generation = self.generation(id.clone())?;
        let on_desktop = self.inner.desktop_id()?.as_deref() == Some(id.as_str());
        let (image, thumb) = self.inner.remove_files(&generation, on_desktop);
        if image.is_some() || thumb.is_some() {
            self.inner.store().set_image_paths(&id, image.as_deref(), thumb.as_deref())?;
            return Err(AutoPaperError::Storage { detail: "a file of this wallpaper couldn't be deleted".into() });
        }
        self.inner.store().delete_generation(&id)
    }

    /// The primary display's size in pixels: the engine picks the image size to request from it (closest
    /// aspect, largest available; free-size providers get no more pixels than this). Hosts call it at launch
    /// and when displays change; until then the engine aims at 3840×2160. `InvalidInput` for a side outside
    /// 1–16384.
    pub fn set_display_hint(&self, width: u32, height: u32) -> Result<()> {
        if !(1..=MAX_RENDER_SIDE).contains(&width) || !(1..=MAX_RENDER_SIDE).contains(&height) {
            return Err(invalid(InvalidInputReason::DisplaySizeInvalid, format!("{width}×{height} isn't a display size")));
        }
        *lock(&self.inner.display) = (width, height);
        Ok(())
    }

    /// Which embedding model memory uses, and whether it is the reduced fallback.
    pub fn memory_status(&self) -> MemoryStatus {
        let embedding_model = self.inner.embedder.model_id().to_string();
        MemoryStatus {
            reduced: embedding_model == HashingEmbedder::MODEL_ID,
            embedding_model,
            problem: self.inner.embedder_problem.clone(),
        }
    }

    /// Deletes images and history. With `keep_memory`, concepts and embeddings stay so the agent
    /// still avoids repeating itself (cleared wallpapers leave History and are never echoed); without, it
    /// forgets everything (taste too). Keywords, settings and this month's spend stay either way.
    /// `current()` is `None` afterwards, but the desktop keeps its picture: the display renders of the wallpaper
    /// on it stay on disk until another wallpaper is shown (then the next prune deletes them).
    /// **Blocking**: deletes every image file.
    pub fn clear_history(&self, keep_memory: bool) -> Result<()> {
        let inner = &self.inner;
        let desktop = inner.desktop_id()?;
        for dir in [&inner.dirs.images, &inner.dirs.thumbs] {
            match fs::remove_dir_all(dir) {
                Ok(()) => {}
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
            fs::create_dir_all(dir)?;
        }
        inner.remove_renders(|id| desktop.as_deref() != Some(id));
        inner.store().clear_history(keep_memory)
    }
}

/// Rust-only API: construction with injected dependencies, and tools for the CLI and tests.
impl Engine {
    async fn make_request(
        &self, trigger: Trigger, echo_id: Option<String>, observer: Option<Arc<dyn ProgressObserver>>, allow_revisit: bool,
    ) -> Result<Shown> {
        let inner = &self.inner;
        let ticket = inner.cancel_ticket();
        let _making = inner.turn(ticket).await?;
        inner.begin(ticket)?;
        inner.start_run(trigger)?;
        let _run = RunGuard::new(inner.console.clone());
        let result = async {
            let original = match (trigger, echo_id) {
                (Trigger::EchoRequest, Some(id)) => Some(inner.echo_original(&id)?),
                (Trigger::EchoRequest, None) => Some(inner.any_echo_original()?),
                _ => None,
            };
            match inner.attempt(trigger, original, observer.as_ref()).await {
                Ok(generation) => Ok(Shown { generation, revisit: None }),
                Err(Failed { error, service_mood: Some(mood), .. }) if allow_revisit => inner.service_fallback(mood, error).await,
                Err(failed) => Err(failed.error),
            }
        }.await;
        inner.finish_run(result.as_ref().map(|shown| Some(&shown.generation)))?;
        result
    }

    /// `open` with its dependencies supplied: a clock, an HTTP client, an embedder and an RNG seed.
    pub fn open_with(config: EngineConfig, secrets: Arc<dyn SecretStore>, deps: Deps) -> Result<Arc<Self>> {
        Self::open_inner(config, secrets, deps, None)
    }

    fn open_inner(
        config: EngineConfig,
        secrets: Arc<dyn SecretStore>,
        deps: Deps,
        embedder_problem: Option<String>,
    ) -> Result<Arc<Self>> {
        if config.data_dir.trim().is_empty() {
            return Err(invalid(InvalidInputReason::Other, "the data directory isn't set"));
        }
        let root = PathBuf::from(&config.data_dir);
        let dirs =
            Dirs { images: root.join("images"), thumbs: root.join("thumbs"), renders: root.join("renders"), root };
        for dir in [&dirs.root, &dirs.images, &dirs.thumbs, &dirs.renders] {
            fs::create_dir_all(dir)?;
        }
        let mut store = Store::open(&dirs.root.join(DATABASE_FILE))?;
        store.interrupt_runs(deps.clock.now())?;
        store.prune_runs(deps.clock.now())?;
        let store = Arc::new(Mutex::new(store));
        let console = Arc::new(RunCapture { active: Mutex::new(None), secrets: Mutex::new(Vec::new()), store: store.clone(), clock: deps.clock.clone() });
        Ok(Arc::new(Self {
            inner: Arc::new(Inner {
                config,
                dirs,
                store,
                console,
                secrets: Arc::new(GuardedSecrets(secrets)),
                clock: deps.clock,
                http: deps.http,
                embedder: deps.embedder,
                rng: Mutex::new(StdRng::seed_from_u64(deps.rng_seed)),
                making: tokio::sync::Mutex::new(()),
                cancels: tokio::sync::watch::Sender::new(0),
                serving: AtomicU64::new(0),
                embedder_problem,
                memory_current: AtomicBool::new(false),
                display: Mutex::new(DEFAULT_DISPLAY),
                stats: Mutex::new(EngineStats::default()),
                detail: Mutex::new(None),
                demo_delay_ms: AtomicU64::new(0),
                appearance: Mutex::new(None),
            }),
        }))
    }

    /// Demo's slow mode (see `DEMO_DELAY_VARIABLE`): a Demo wallpaper takes about `delay`; zero turns it off.
    pub fn set_demo_delay(&self, delay: Duration) {
        let millis = delay.min(demo::MAX_DELAY).as_millis();
        self.inner.demo_delay_ms.store(u64::try_from(millis).unwrap_or(u64::MAX), Ordering::SeqCst);
    }

    pub fn data_dir(&self) -> &Path {
        &self.inner.dirs.root
    }

    /// The id of the embedding model memory uses.
    pub fn embedding_model(&self) -> String {
        self.inner.embedder.model_id().to_string()
    }

    pub fn stats(&self) -> EngineStats {
        lock(&self.inner.stats).clone()
    }

    /// A dry run of composing: one text call, every candidate checked and scored as `generate` would, and
    /// the one it would choose; nothing painted or stored except the text call's cost.
    pub async fn compose_preview(&self) -> Result<ComposePreview> {
        let inner = &self.inner;
        let ticket = inner.cancel_ticket();
        let _making = inner.turn(ticket).await?;
        inner.begin(ticket)?;
        let (mut job, text, _image) = inner.prepare(Trigger::Manual, None)?;
        inner.ensure_memory_embedded().await?;
        let basis = Arc::new(inner.basis(&job)?);
        let context = inner.compose_context(&job, false, &basis.taste)?;
        // The text call's cost is added to the month's spend as soon as it answers.
        let candidates = inner.compose_once(&mut job, text.as_ref(), &context, &basis, None).await?;
        let chosen = best_novel(&candidates).or_else(|| least_similar(&candidates));
        let text_model = candidates.first().map_or_else(|| job.text_model.clone(), |c| c.model.clone());
        Ok(ComposePreview {
            candidates: candidates.iter().map(|candidate| preview(candidate, &basis.memory)).collect(),
            chosen,
            threshold: basis.policy.threshold,
            embedding_model: inner.embedder.model_id().to_string(),
            text_model,
            cost_microusd: job.spent,
        })
    }
}

/// The real embedding model from `model_dir`, else the hashing fallback (logged): memory keeps working,
/// but only catches near-verbatim repeats until the model is present.
pub fn default_embedder(model_dir: &Path) -> Arc<dyn Embedder> {
    load_embedder(model_dir).0
}

/// `default_embedder`, plus why the real model couldn't be loaded (for `memory_status`).
fn load_embedder(model_dir: &Path) -> (Arc<dyn Embedder>, Option<String>) {
    match CandleEmbedder::load(model_dir) {
        Ok(embedder) => (Arc::new(embedder), None),
        Err(error) => {
            tracing::warn!(%error, "the embedding model isn't available; memory uses the hashing fallback");
            (Arc::new(HashingEmbedder), Some(error.to_string()))
        }
    }
}

/// The host's secret store, read so that a failing implementation can't take the core down: UniFFI turns a
/// foreign exception into a panic, which is caught here and read as "no key" (`MissingKey`).
struct GuardedSecrets(Arc<dyn SecretStore>);

impl GuardedSecrets {
    fn guarded<T>(&self, what: &str, call: impl FnOnce() -> T) -> Option<T> {
        match catch_unwind(AssertUnwindSafe(call)) {
            Ok(value) => Some(value),
            Err(_) => {
                tracing::warn!(what, "the secret store failed");
                None
            }
        }
    }
}

impl SecretStore for GuardedSecrets {
    fn get(&self, account: String) -> Option<String> {
        self.guarded("get", || self.0.get(account)).flatten()
    }

    fn set(&self, account: String, value: String) {
        self.guarded("set", || self.0.set(account, value));
    }

    fn delete(&self, account: String) {
        self.guarded("delete", || self.0.delete(account));
    }
}

// ── One wallpaper being made ────────────────────────────────────────────────────────────────

/// A past generation to echo.
#[derive(Debug, Clone)]
struct EchoOriginal {
    id: String,
    created_at: i64,
    concept: Concept,
}

#[derive(Debug, Clone)]
struct EchoJob {
    original: EchoOriginal,
    axes: Vec<EchoAxis>,
    /// `echo::age_text` of the original's age.
    age: String,
}

/// A job and the providers it uses.
type Prepared = (Job, Arc<dyn TextProvider>, Arc<dyn ImageProvider>);

/// What was decided and spent so far, so a failure can be recorded as faithfully as a success.
struct Job {
    trigger: Trigger,
    started: i64,
    settings: Settings,
    keywords: Vec<Keyword>,
    /// The active mood when it started, and its name then.
    mood_id: String,
    mood_name: String,
    echo: Option<EchoJob>,
    text_kind: ProviderKind,
    /// Resolved (the provider's default when the setting is blank); the answering model replaces it.
    text_model: String,
    /// Where the timings of this job's calls are filed (`origin_of`).
    text_origin: String,
    image_origin: String,
    /// The painter's sampler steps, when it knows them.
    image_steps: Option<u32>,
    image_kind: ProviderKind,
    image_model: String,
    /// The size requested from the image provider.
    width: u32,
    height: u32,
    /// Micro-USD spent on this attempt so far (text calls, the image). Each call's cost is added to the
    /// month's spend as soon as it answers, so a dropped call still counts.
    spent: u64,
    /// Images paid for (0 or 1).
    images: u32,
    /// Text-model answers that could be read (had candidates), to tell "unreadable" from "unusable".
    answers_read: u32,
    /// What made this job's unusable candidates unusable, every one counted: retries ask the model to put the
    /// most common right, and with no usable candidate at all the error names the most common.
    problems: Vec<Problem>,
    chosen: Option<Evaluated>,
    /// The painting failed because of how painting is set up (`needs_setup`), which trying again can't fix.
    needs_setup: bool,
}

/// A wallpaper that couldn't be made: the error, and whether how painting is set up must change first (`needs_setup`).
struct Failed {
    error: AutoPaperError,
    needs_setup: bool,
    service_mood: Option<String>,
}

impl From<AutoPaperError> for Failed {
    fn from(error: AutoPaperError) -> Self {
        let needs_setup = matches!(error, AutoPaperError::PaintingFailed { .. });
        Self { error, needs_setup, service_mood: None }
    }
}

/// One candidate, assessed.
#[derive(Debug, Clone)]
struct Evaluated {
    composed: Composed,
    problems: Vec<Problem>,
    embedding: Vec<f32>,
    report: NoveltyReport,
    /// Echoes: cosine to the original.
    echo_cosine: Option<f32>,
    /// Echoes: that cosine is within the embedding model's echo band.
    echo_in_band: Option<bool>,
    /// Echoes: at least one requested axis visibly changed its field.
    echo_changed: Option<bool>,
    taste: f32,
    score: f32,
    /// Passes the checks (and, for echoes, the band and the visible change).
    valid: bool,
    novel: bool,
    /// The text model that wrote it.
    model: String,
    /// Chosen as the least similar valid candidate: nothing was novel enough, even after asking again.
    least_similar: bool,
}

/// Everything a novelty check needs, loaded once per compose.
struct Basis {
    memory: Vec<MemoryRow>,
    taste: Taste,
    policy: NoveltyPolicy,
    calibration: Calibration,
    now: i64,
    surprise: f32,
}

/// The active run is serialized by `making`. HTTP clients carry its id so a late cancellation response
/// can never attach to the next run. Console writes contain no authentication headers.
struct RunCapture {
    active: Mutex<Option<RunRecord>>,
    // Kept in memory only, so even a provider echoing an opaque credential cannot persist it.
    secrets: Mutex<Vec<String>>,
    store: Arc<Mutex<Store>>,
    clock: Arc<dyn Clock>,
}

/// A host dropping the future is an interruption too, even if the app itself stays open.
struct RunGuard {
    console: Arc<RunCapture>,
    id: Option<String>,
}

impl RunGuard {
    fn new(console: Arc<RunCapture>) -> Self { Self { id: console.id(), console } }
}

impl Drop for RunGuard {
    fn drop(&mut self) {
        let mut active = lock(&self.console.active);
        if active.as_ref().map(|run| &run.id) != self.id.as_ref() { return }
        if let Some(mut run) = active.take() {
            run.status = RunStatus::Interrupted;
            run.finished_at = Some(self.console.clock.now());
            run.detail = "This run was interrupted before its final outcome was recorded. A provider may still have billed an unfinished request.".into();
            if let Err(error) = lock(&self.console.store).save_run(&run) {
                tracing::warn!(%error, "couldn't record the interrupted run");
            }
        }
    }
}

impl RunCapture {
    fn id(&self) -> Option<String> {
        lock(&self.active).as_ref().map(|run| run.id.clone())
    }

    fn text(&self, detail: &str) -> String {
        console_text_with_secrets(detail, &lock(&self.secrets))
    }

    fn remember_secrets(&self, id: Option<&str>, values: &[String]) {
        let active = lock(&self.active);
        if active.as_ref().map(|run| run.id.as_str()) != id { return }
        let mut secrets = lock(&self.secrets);
        for value in values {
            if !secrets.contains(value) { secrets.push(value.clone()); }
        }
    }

    fn event(&self, id: Option<&str>, stage: &str, provider: Option<ProviderKind>, model: &str, kind: &str, detail: &str) {
        let mut active = lock(&self.active);
        let Some(run) = active.as_mut() else { return };
        if id.is_some_and(|id| run.id != id) { return }
        // A bound on a single run as well as the age/count retention bound. Normal runs are far smaller.
        let size: usize = run.events.iter().map(|event| event.detail.len()).sum();
        if run.events.last().is_some_and(|event| event.kind == "truncated") { return }
        if run.events.len() >= 255 || size >= 448 * 1024 {
            run.events.push(RunEvent { at: self.clock.now(), stage: stage.into(), provider: None,
                model: String::new(), kind: "truncated".into(),
                detail: "Further details were omitted because this run reached the Console's trace size limit.".into() });
            if let Err(error) = lock(&self.store).save_run(run) {
                tracing::warn!(%error, "couldn't record Console truncation");
            }
            return;
        }
        run.events.push(RunEvent {
            at: self.clock.now(), stage: stage.into(), provider,
            model: self.text(model), kind: kind.into(), detail: self.text(detail),
        });
        if kind == "stage" {
            run.detail = match stage {
                "CheckingServices" => "Checking services…",
                "Composing" => "Writing ideas…",
                "CheckingMemory" => "Checking ideas against memory…",
                "Generating" => "Painting…",
                "Downloading" => "Reading the image…",
                "Rendering" => "Preparing the wallpaper…",
                "Done" => "Wallpaper ready.",
                _ => "Working…",
            }.into();
        }
        if let Err(error) = lock(&self.store).save_run(run) {
            tracing::warn!(%error, "couldn't update the Console run");
        }
    }
}

/// Capture actual mapped API payloads, including provider-internal retries, without credentials or pixels.
struct ConsoleHttp {
    http: Arc<dyn HttpClient>,
    console: Arc<RunCapture>,
    run_id: Option<String>,
    provider: ProviderKind,
    stage: &'static str,
    model: String,
    seen_gets: Mutex<HashSet<String>>,
}

#[async_trait::async_trait]
impl HttpClient for ConsoleHttp {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse> {
        let secrets: Vec<String> = request.headers.iter()
            .filter(|(name, _)| net::CREDENTIAL_HEADERS.iter().any(|header| header.eq_ignore_ascii_case(name)))
            .flat_map(|(_, value)| [value.clone(), value.strip_prefix("Bearer ").unwrap_or(value).to_string()])
            .filter(|value| !value.is_empty()).collect();
        let capture = self.run_id.is_some();
        let first = request.method != HttpMethod::Get || lock(&self.seen_gets).insert(request.url.clone());
        let address = url::Url::parse(&request.url).map(|url| format!("{}{}", url.origin().ascii_serialization(), url.path())).unwrap_or_default();
        let method = if request.method == HttpMethod::Post { "POST" } else { "GET" };
        if capture {
            self.console.remember_secrets(self.run_id.as_deref(), &secrets);
        }
        if capture && first {
            // Strip URL credentials and query strings; no headers enter the record at all.
            let body = request.body.as_deref().map(|body| console_body_with_secrets(body, &secrets)).unwrap_or_default();
            self.console.event(self.run_id.as_deref(), self.stage, Some(self.provider), &self.model, "request",
                &format!("{method} {address}\nTimeout: {} s\n{body}", request.timeout_secs));
        }
        let started = tokio::time::Instant::now();
        let response = self.http.send(request).await;
        // Repeated empty ComfyUI history polls carry no new outcome. Retain the first poll and every result/error.
        let empty_poll = !first && response.as_ref().is_ok_and(|response| response.status == 200 && response.body == b"{}");
        if capture && !empty_poll {
            let detail = match &response {
                Ok(response) => format!("{method} {address}\nHTTP {} · {:.2} s\n{}", response.status, started.elapsed().as_secs_f64(), console_body_with_secrets(&response.body, &secrets)),
                Err(error) => format!("{method} {address}\n{error} · {:.2} s", started.elapsed().as_secs_f64()),
            };
            self.console.event(self.run_id.as_deref(), self.stage, Some(self.provider), &self.model, "response", &detail);
        }
        response
    }

    async fn open_socket(&self, request: HttpRequest) -> Result<Box<dyn Socket>> {
        self.http.open_socket(request).await
    }
}

fn console_text(text: &str) -> String {
    let text = net::redact(text);
    let mut bounded: String = text.chars().filter(|ch| !ch.is_control() || matches!(ch, '\n' | '\t')).collect();
    if let Some((index, _)) = bounded.char_indices().find(|(index, ch)| index + ch.len_utf8() > 65_536) {
        bounded.truncate(index);
        bounded.push_str("\n[Console detail truncated at 65,536 bytes]");
    }
    bounded
}

fn console_text_with_secrets(text: &str, secrets: &[String]) -> String {
    let mut text = text.to_owned();
    for secret in secrets { text = text.replace(secret, "[redacted]"); }
    console_text(&text)
}

/// Preserve sub-cent estimates: rounding either term to cents can make a valid budget block look wrong.
fn console_money(microusd: u64) -> String {
    let mut fraction = format!("{:06}", microusd % 1_000_000);
    while fraction.len() > 2 && fraction.ends_with('0') { fraction.pop(); }
    format!("${}.{}", microusd / 1_000_000, fraction)
}

fn console_body_with_secrets(bytes: &[u8], secrets: &[String]) -> String {
    fn sanitize(value: &mut Value, secrets: &[String]) {
        match value {
            Value::Object(fields) => {
                for (key, value) in fields {
                    let key = key.to_ascii_lowercase().replace(['-', '_'], "");
                    if matches!(key.as_str(), "authorization" | "xgoogapikey" | "apikey" | "password" | "secret" | "token" | "accesstoken" | "refreshtoken" | "cookie" | "thoughtsignature") {
                        *value = Value::String("[redacted]".into());
                    } else if matches!(key.as_str(), "b64json" | "inlinedata" | "imagedata") {
                        *value = Value::String("[image bytes omitted]".into());
                    } else { sanitize(value, secrets); }
                }
            }
            Value::Array(values) => values.iter_mut().for_each(|value| sanitize(value, secrets)),
            Value::String(text) if text.starts_with("data:") => *text = "[data bytes omitted]".into(),
            Value::String(text) => {
                if let Ok(mut address) = url::Url::parse(text)
                    && matches!(address.scheme(), "http" | "https")
                {
                    let _ = address.set_username("");
                    let _ = address.set_password(None);
                    address.set_query(None);
                    address.set_fragment(None);
                    *text = address.to_string();
                }
                *text = net::redact(text);
                for secret in secrets { *text = text.replace(secret, "[redacted]"); }
            }
            _ => {}
        }
    }
    match serde_json::from_slice::<Value>(bytes) {
        Ok(mut value) => {
            sanitize(&mut value, secrets);
            console_text(&serde_json::to_string_pretty(&value).unwrap_or_default())
        }
        Err(_) => format!("[non-JSON response: {} bytes]", bytes.len()),
    }
}

impl Inner {
    fn budget_status(&self) -> Result<BudgetStatus> {
        let settings = self.store().settings()?;
        let month = schedule::month_key(self.now());
        let (spent, _) = self.store().spend(&month)?;
        let next = self.estimate_cost(&settings);
        let blocked = next > 0 && !schedule::budget_allows(settings.monthly_budget_cents, spent, next);
        let message = if blocked {
            format!("New wallpapers are paused by your monthly budget: {} estimated spent of a ${:.2} limit; the next wallpaper is estimated at {}. Your current wallpaper stays in place. The budget resets next month, or you can change it in Budget settings.",
                console_money(spent), settings.monthly_budget_cents.unwrap_or(0) as f64 / 100.0, console_money(next))
        } else { String::new() };
        Ok(BudgetStatus { month, spent_microusd: spent, budget_cents: settings.monthly_budget_cents, next_cost_microusd: next, blocked, message })
    }

    fn start_run(&self, trigger: Trigger) -> Result<()> {
        let settings = self.store().settings()?;
        let mood = self.store().active_mood()?;
        let mut keywords = snapshot(&mood.keywords);
        for keyword in &mut keywords { keyword.text = console_text(&keyword.text); }
        let run = RunRecord {
            id: Uuid::now_v7().to_string(), started_at: self.now(), finished_at: None, trigger,
            status: RunStatus::Running, mood_name: console_text(&mood.name), keywords,
            surprise: settings.surprise, text_provider: settings.text_provider.kind,
            text_model: console_text(model_or(&settings.text_provider.model, &registry::default_model(settings.text_provider.kind, ProviderJob::Concepts))),
            image_provider: settings.image_provider.kind,
            image_model: console_text(model_or(&settings.image_provider.model, &registry::default_model(settings.image_provider.kind, ProviderJob::Images))),
            generation_id: None, detail: "Preparing this run.".into(), cost_microusd: 0, events: Vec::new(),
        };
        self.store().save_run(&run)?;
        self.store().prune_runs(self.now())?;
        lock(&self.console.secrets).clear();
        *lock(&self.console.active) = Some(run);
        Ok(())
    }

    fn finish_run(&self, outcome: std::result::Result<Option<&Generation>, &AutoPaperError>) -> Result<()> {
        let mut active = lock(&self.console.active);
        let Some(mut run) = active.take() else { return Ok(()) };
        run.finished_at = Some(self.now());
        match outcome {
            Ok(Some(generation)) => {
                // A scheduled fallback may have returned an older image after this run failed.
                if run.status == RunStatus::Running {
                    run.status = RunStatus::Succeeded;
                    run.detail = self.console.text(&format!("Generated “{}”.", generation.concept.title));
                    run.generation_id = Some(generation.id.clone());
                } else {
                    run.detail.push_str(" A saved wallpaper was shown instead.");
                }
            }
            Ok(None) => { run.status = RunStatus::Blocked; run.detail = "No wallpaper was generated.".into(); }
            Err(error) => {
                run.status = match error {
                    AutoPaperError::Cancelled => RunStatus::Cancelled,
                    AutoPaperError::MissingKey { .. } | AutoPaperError::BudgetReached { .. } | AutoPaperError::InvalidInput { .. } | AutoPaperError::Unsupported { .. } => RunStatus::Blocked,
                    _ => RunStatus::Failed,
                };
                run.detail = if matches!(error, AutoPaperError::BudgetReached { .. }) {
                    self.budget_status()?.message
                } else { self.console.text(&error.to_string()) };
                if !run.events.iter().any(|event| event.kind == "request") {
                    run.detail.push_str(" No network request was sent.");
                }
            }
        }
        self.store().save_run(&run)?;
        self.store().prune_runs(self.now())?;
        Ok(())
    }

    fn run_deps(&self, settings: &Settings, selection: &ProviderSelection, stage: &'static str) -> ProviderDeps {
        let mut deps = self.deps(settings);
        deps.http = Arc::new(ConsoleHttp {
            http: self.http.clone(), console: self.console.clone(), run_id: self.console.id(),
            provider: selection.kind, stage, model: model_or(&selection.model, &registry::default_model(selection.kind, if stage == "Writing" { ProviderJob::Concepts } else { ProviderJob::Images })).to_string(),
            seen_gets: Mutex::new(HashSet::new()),
        });
        deps
    }

    fn run_cost(&self, job: &Job) {
        let mut active = lock(&self.console.active);
        if let Some(run) = active.as_mut() {
            run.cost_microusd = job.spent;
            if let Some(chosen) = &job.chosen { run.text_model = self.console.text(&chosen.model); }
            run.image_model = self.console.text(&job.image_model);
            if let Err(error) = self.store().save_run(run) { tracing::warn!(%error, "couldn't record run cost"); }
        }
    }

    fn store(&self) -> MutexGuard<'_, Store> {
        lock(&self.store)
    }

    fn now(&self) -> i64 {
        self.clock.now()
    }

    fn next_u64(&self) -> u64 {
        lock(&self.rng).next_u64()
    }

    fn stat(&self, update: impl FnOnce(&mut EngineStats)) {
        update(&mut lock(&self.stats));
    }

    /// Noted by a request before it waits for its turn: a `cancel` after this stops it.
    fn cancel_ticket(&self) -> u64 {
        *self.cancels.borrow()
    }

    /// Waits for this request's turn (one generation at a time); `Cancelled` as soon as the person cancels.
    async fn turn(&self, ticket: u64) -> Result<tokio::sync::MutexGuard<'_, ()>> {
        tokio::select! {
            biased;
            () = self.cancelled_after(ticket) => Err(AutoPaperError::Cancelled),
            making = self.making.lock() => Ok(making),
        }
    }

    /// A request got its turn: `Cancelled` if the person cancelled while it waited.
    fn begin(&self, ticket: u64) -> Result<()> {
        self.serving.store(ticket, Ordering::SeqCst);
        self.check_cancel()
    }

    fn check_cancel(&self) -> Result<()> {
        if *self.cancels.borrow() == self.serving.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err(AutoPaperError::Cancelled)
        }
    }

    /// Resolves once a `cancel` has come after `ticket` (at once if one already has).
    async fn cancelled_after(&self, ticket: u64) {
        let mut cancels = self.cancels.subscribe();
        // `wait_for` looks at the current count first, so a cancel before this call isn't missed. It fails only
        // once the sender is gone, which can't happen while `self` is borrowed.
        let closed = cancels.wait_for(|&count| count != ticket).await.is_err();
        if closed {
            std::future::pending::<()>().await;
        }
    }

    /// A provider request, dropped the moment the person cancels (`Cancelled`): a hung or slow request doesn't
    /// hold the cancel up until it times out. An answer that is ready when the cancel comes is taken, so what it
    /// cost is recorded; the cancel then takes effect at the next stage boundary.
    async fn unless_cancelled<T>(&self, request: impl Future<Output = Result<T>>) -> Result<T> {
        let ticket = self.serving.load(Ordering::SeqCst);
        tokio::select! {
            biased;
            result = request => result,
            () = self.cancelled_after(ticket) => Err(AutoPaperError::Cancelled),
        }
    }

    // ── Providers and prices ────────────────────────────────────────────────────────────────

    fn deps(&self, settings: &Settings) -> ProviderDeps {
        ProviderDeps {
            http: self.http.clone(),
            secrets: self.secrets.clone(),
            comfyui_workflow: settings.comfyui_workflow.clone(),
        }
    }

    /// Demo is seeded from the engine's RNG (reproducible runs) and slowed when slow mode is on; everything else
    /// comes from the registry.
    fn text_provider(&self, selection: &ProviderSelection, settings: &Settings) -> Result<Arc<dyn TextProvider>> {
        match selection.kind {
            ProviderKind::Demo => Ok(Arc::new(self.demo(self.next_u64()))),
            _ => registry::text_provider(selection, &self.deps(settings)),
        }
    }

    fn image_provider(&self, selection: &ProviderSelection, settings: &Settings) -> Result<Arc<dyn ImageProvider>> {
        match selection.kind {
            ProviderKind::Demo => Ok(Arc::new(self.demo(self.next_u64()))),
            _ => registry::image_provider(selection, &self.deps(settings)),
        }
    }

    /// A provider only asked about (models, sizes, steps), never used: Demo without touching the engine's RNG, so
    /// asking for estimates doesn't change what a seeded run makes.
    fn quiet_text_provider(&self, selection: &ProviderSelection, settings: &Settings) -> Result<Arc<dyn TextProvider>> {
        match selection.kind {
            ProviderKind::Demo => Ok(Arc::new(self.demo(0))),
            _ => registry::text_provider(selection, &self.deps(settings)),
        }
    }

    fn quiet_image_provider(&self, selection: &ProviderSelection, settings: &Settings) -> Result<Arc<dyn ImageProvider>> {
        match selection.kind {
            ProviderKind::Demo => Ok(Arc::new(self.demo(0))),
            _ => registry::image_provider(selection, &self.deps(settings)),
        }
    }

    fn demo(&self, seed: u64) -> Demo {
        Demo::new(seed).with_delay(Duration::from_millis(self.demo_delay_ms.load(Ordering::SeqCst)))
    }

    // ── Timings ─────────────────────────────────────────────────────────────────────────────

    /// Stores one finished call's timing (logged if it can't be: it's only history).
    fn record_timing(&self, mut timing: Timing, response_model: &str) {
        let run_id = self.console.id();
        let actual_model = self.console.text(&answered_model(response_model, &timing.model));
        if run_id.is_some() { timing.model = self.console.text(&timing.model); }
        if let Err(error) = self.store().add_timing_for_run(&timing, run_id.as_deref(), Some(&actual_model)) {
            tracing::warn!(%error, "couldn't record how long a call took");
        }
    }

    /// `perf::estimate` over the recorded calls of this kind.
    #[allow(clippy::too_many_arguments)]
    fn estimate_secs(
        &self,
        job: ProviderJob,
        kind: ProviderKind,
        origin: &str,
        model: &str,
        width: u32,
        height: u32,
        steps: Option<u32>,
    ) -> Result<Option<f64>> {
        let timings = self.store().timings(job, kind, origin, model)?;
        Ok(perf::estimate(&timings, self.now(), width, height, steps))
    }

    /// Seconds a whole wallpaper takes with these settings: the writing and painting estimates (each 0 when
    /// nothing is recorded yet).
    fn wallpaper_secs(&self, settings: &Settings) -> f64 {
        let text = self.quiet_text_provider(&settings.text_provider, settings).ok().and_then(|provider| {
            let model = model_or(&settings.text_provider.model, provider.default_model()).to_string();
            let origin = origin_of(&settings.text_provider);
            self.estimate_secs(ProviderJob::Concepts, provider.kind(), &origin, &model, 0, 0, None).ok().flatten()
        });
        let image = self.quiet_image_provider(&settings.image_provider, settings).ok().and_then(|provider| {
            let model = model_or(&settings.image_provider.model, provider.default_model()).to_string();
            let (width, height) = self.request_size(provider.as_ref(), &model);
            let steps = provider.steps(&settings.image_provider.model);
            let origin = origin_of(&settings.image_provider);
            self.estimate_secs(ProviderJob::Images, provider.kind(), &origin, &model, width, height, steps).ok().flatten()
        });
        text.unwrap_or(0.0) + image.unwrap_or(0.0)
    }

    // ── Progress ────────────────────────────────────────────────────────────────────────────

    /// Reports a stage to the call's observer and the detail observer (`Done` as complete).
    fn stage(&self, observer: Option<&Arc<dyn ProgressObserver>>, stage: ProgressStage) {
        self.console.event(None, &format!("{stage:?}"), None, "", "stage", "");
        progress(observer, stage);
        let (fraction, seconds_left) = if stage == ProgressStage::Done { (Some(1.0), Some(0)) } else { (None, None) };
        self.detail(ProgressDetail { stage, fraction, seconds_left });
    }

    /// Reports to the host's detail observer, if any (one that fails is logged and ignored, like `progress`).
    fn detail(&self, detail: ProgressDetail) {
        let observer = lock(&self.detail).clone();
        if let Some(observer) = observer
            && catch_unwind(AssertUnwindSafe(|| observer.on_progress_detail(detail))).is_err()
        {
            tracing::warn!(stage = ?detail.stage, "the progress detail observer failed");
        }
    }

    /// The painting call (dropped on cancel, as `unless_cancelled`), reporting its fraction and seconds left to the
    /// detail observer: on each step the painter reports and on a one-second tick, whenever they changed.
    /// `reports_steps`: the painter tells its sampler steps (it knows how many it runs), so time alone fills only
    /// part of the bar before they come (`PaintTracker`).
    async fn paint(&self, image: &dyn ImageProvider, mut request: ImageRequest, reports_steps: bool) -> Result<ImageResponse> {
        if lock(&self.detail).is_none() {
            return self.unless_cancelled(image.generate(request)).await;
        }
        let mut tracker = PaintTracker::new(tokio::time::Instant::now(), request.expected_secs, reports_steps);
        let (sender, mut steps) = tokio::sync::mpsc::unbounded_channel::<(tokio::time::Instant, PaintStep)>();
        request.progress = Some(PaintProgress(Arc::new(move |step| {
            let _ = sender.send((tokio::time::Instant::now(), step));
        })));
        let painting = self.unless_cancelled(image.generate(request));
        tokio::pin!(painting);
        let mut tick = tokio::time::interval(DETAIL_TICK);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // The stage itself already said "no detail yet".
        let mut said = (None, None);
        loop {
            tokio::select! {
                biased;
                result = &mut painting => return result,
                Some((at, step)) = steps.recv() => tracker.step(at, step.done, step.total),
                _ = tick.tick() => {}
            }
            let (fraction, seconds_left) = tracker.at(tokio::time::Instant::now());
            let shown = (fraction.map(|fraction| (fraction * 100.0).floor() as u32), seconds_left);
            if shown != said {
                said = shown;
                self.detail(ProgressDetail { stage: ProgressStage::Generating, fraction, seconds_left });
            }
        }
    }

    /// The size to ask for: closest to the display's aspect, largest available — but for free-size
    /// providers no more pixels than the display has (they'd only be scaled away).
    fn request_size(&self, provider: &dyn ImageProvider, model: &str) -> (u32, u32) {
        let (width, height) = *lock(&self.display);
        let mut caps = provider.capabilities(model);
        if let Some(free) = caps.free_size.as_mut() {
            free.max_pixels = free.max_pixels.min(u64::from(width) * u64::from(height));
        }
        imaging::choose_size(&caps, width, height)
    }

    /// Estimated cost of one more wallpaper: a typical compose call plus one image at the size these
    /// settings would request. Only hosted providers cost anything; a provider that can't be built costs 0.
    fn estimate_cost(&self, settings: &Settings) -> u64 {
        let paid = |kind: ProviderKind| matches!(kind, ProviderKind::OpenAi | ProviderKind::Google);
        let text = if paid(settings.text_provider.kind) {
            self.text_provider(&settings.text_provider, settings).map_or(0, |provider| {
                let model = model_or(&settings.text_provider.model, provider.default_model());
                pricing::text_cost(provider.kind(), model, &pricing::TYPICAL_COMPOSE_USAGE)
            })
        } else {
            0
        };
        let image = if paid(settings.image_provider.kind) {
            self.image_provider(&settings.image_provider, settings).map_or(0, |provider| {
                let model = model_or(&settings.image_provider.model, provider.default_model());
                let (width, height) = self.request_size(provider.as_ref(), model);
                image_price(provider.kind(), model, width, height, settings.image_quality)
            })
        } else {
            0
        };
        text.saturating_add(image)
    }

    /// Free wallpapers always fit; otherwise this month's spend plus `estimate` must stay within the cap.
    fn budget_allows(&self, settings: &Settings, now: i64, estimate: u64) -> Result<bool> {
        if estimate == 0 {
            return Ok(true);
        }
        let (spent, _) = self.store().spend(&schedule::month_key(now))?;
        Ok(schedule::budget_allows(settings.monthly_budget_cents, spent, estimate))
    }

    // ── Schedule state ──────────────────────────────────────────────────────────────────────

    fn state_i64(&self, key: &str) -> Result<Option<i64>> {
        Ok(self.store().state_get(key)?.and_then(|value| value.as_i64()))
    }

    fn set_state(&self, key: &str, value: Option<i64>) -> Result<()> {
        self.store().state_set(key, &value.map_or(Value::Null, Value::from))
    }

    /// When the next scheduled wallpaper is due: one interval after the newest new wallpaper (any trigger) or
    /// slot skipped over budget. A last slot later than `now` means the clock was set back:
    /// the slot is moved to `now` and stored, so the wait is one interval rather than until the clock
    /// catches up.
    fn next_due_at(&self, settings: &Settings, now: i64) -> Result<Option<i64>> {
        let last = match self.state_i64(STATE_LAST_SLOT_AT)? {
            Some(at) => Some(at),
            None => {
                // A database with no record yet. Bound first: the store's guard would otherwise live through
                // `state_i64` and deadlock.
                let last_new = self.store().last_new_at()?;
                last_new.max(self.state_i64(STATE_SLOT_SKIPPED_AT)?)
            }
        };
        let last = match last {
            Some(at) if at > now => {
                self.fill_slot(now, now)?;
                Some(now)
            }
            other => other,
        };
        let backoff_until = self.state_i64(STATE_BACKOFF_UNTIL)?;
        let lead = self.state_i64(STATE_SLOT_LEAD)?.unwrap_or(0);
        Ok(schedule::next_due_after(settings, last, lead, backoff_until, now))
    }

    /// The scheduled slot due at `due` is filled at `at` (a wallpaper started then, or the run was skipped or
    /// cancelled then): the next is one interval after `due`, or after `at` when that's later.
    fn fill_slot(&self, at: i64, due: i64) -> Result<()> {
        self.set_state(STATE_LAST_SLOT_AT, Some(at))?;
        self.set_state(STATE_SLOT_LEAD, Some(due.saturating_sub(at).max(0)))
    }

    /// (due, start) of the next scheduled wallpaper: `next_due_at`, and that less a wallpaper's estimated duration
    /// (at most half an interval, never before the backoff ends). `None` when paused or manual.
    fn schedule_at(&self, settings: &Settings, now: i64) -> Result<Option<(i64, i64)>> {
        let Some(due) = self.next_due_at(settings, now)? else { return Ok(None) };
        let interval = settings.cadence.interval_secs().unwrap_or(0);
        let estimate = self.wallpaper_secs(settings);
        let lead = if estimate > 0.0 { (estimate.round() as i64).min(interval / 2).max(0) } else { 0 };
        let backoff_until = self.state_i64(STATE_BACKOFF_UNTIL)?.unwrap_or(i64::MIN);
        Ok(Some((due, due.saturating_sub(lead).max(backoff_until).min(due))))
    }

    /// Counts a failed scheduled attempt and backs off from now (the attempt itself can take minutes), at
    /// least as long as a rate limit's `Retry-After`; returns the failures in a row.
    fn record_scheduled_failure(&self, error: &AutoPaperError) -> Result<u32> {
        let before = self.state_i64(STATE_FAILURES)?.and_then(|n| u32::try_from(n).ok()).unwrap_or(0);
        let failures = before.saturating_add(1);
        let mut wait = schedule::backoff_secs(failures);
        if let AutoPaperError::RateLimited { retry_after_secs, .. } = error {
            wait = wait.max(i64::from(*retry_after_secs));
        }
        self.set_state(STATE_FAILURES, Some(i64::from(failures)))?;
        self.set_state(STATE_BACKOFF_UNTIL, Some(self.now().saturating_add(wait)))?;
        Ok(failures)
    }

    /// A wallpaper was made: the providers work, so any backoff ends.
    fn record_success(&self) -> Result<()> {
        self.set_state(STATE_FAILURES, Some(0))?;
        self.set_state(STATE_BACKOFF_UNTIL, None)
    }

    /// `run_if_due` once it's due (or started early for `due`) and this request has its turn. Whatever happens to
    /// it fills the slot due at `due`, not at its early start.
    async fn run_due(
        self: &Arc<Self>,
        ticket: u64,
        settings: &Settings,
        now: i64,
        due: i64,
        observer: Option<&Arc<dyn ProgressObserver>>,
    ) -> Result<Option<Shown>> {
        self.begin(ticket)?;
        if !self.budget_allows(settings, now, self.estimate_cost(settings))? {
            self.fill_slot(now, due)?;
            return Err(budget_reached(settings));
        }
        let original = self.scheduled_echo_original(settings, now)?;
        match self.attempt(Trigger::Scheduled, original, observer).await {
            Ok(generation) => {
                if let Err(error) = self.fill_slot(generation.created_at, due) {
                    tracing::warn!(%error, "couldn't restart the schedule");
                }
                Ok(Some(Shown { generation, revisit: None }))
            }
            Err(Failed { error: AutoPaperError::Cancelled, .. }) => Err(AutoPaperError::Cancelled),
            Err(Failed { error, service_mood: Some(mood), .. }) => {
                self.record_scheduled_failure(&error)?;
                self.service_fallback(mood, error).await.map(Some)
            }
            Err(Failed { error, needs_setup: true, .. }) => {
                // ComfyUI couldn't run its workflow: trying again on the timer would only pay for another idea and
                // load the models again to fail the same way. No liked wallpaper stands in (the host shows the
                // problem and its fix), and the slot counts as filled: the next try is one interval later, or when
                // the person asks.
                self.record_scheduled_failure(&error)?;
                self.fill_slot(now, due)?;
                Err(error)
            }
            Err(Failed { error, .. }) => {
                let failures = self.record_scheduled_failure(&error)?;
                match revisit_reason(&error) {
                    Some(reason) if failures == 1 => self.fallback(settings, reason, error).map(Some),
                    _ => Err(error),
                }
            }
        }
    }

    /// A liked wallpaper instead of `error`, when the settings ask for one and there is one.
    fn fallback(&self, settings: &Settings, reason: RevisitReason, error: AutoPaperError) -> Result<Shown> {
        if let Some(run) = lock(&self.console.active).as_mut() {
            run.status = RunStatus::Failed;
            run.detail = self.console.text(&error.to_string());
        }
        if settings.fallback != Fallback::RevisitLiked {
            return Err(error);
        }
        match self.revisit(reason) {
            Ok(shown) => {
                tracing::info!(?reason, error = %net::redact(&error.to_string()), "revisiting a liked wallpaper");
                Ok(shown)
            }
            Err(AutoPaperError::NothingToRevisit) => Err(error),
            Err(other) => Err(other),
        }
    }

    fn revisit(&self, reason: RevisitReason) -> Result<Shown> {
        let (liked, current) = {
            let store = self.store();
            (store.liked_with_images()?, store.current()?.map(|generation| generation.id))
        };
        let on_disk: Vec<Generation> = liked
            .into_iter()
            .filter(|generation| generation.image_path.as_deref().is_some_and(|path| Path::new(path).is_file()))
            .collect();
        let pick =
            on_disk.iter().find(|generation| current.as_deref() != Some(generation.id.as_str())).or(on_disk.first());
        pick.cloned()
            .map(|generation| Shown { generation, revisit: Some(reason) })
            .ok_or(AutoPaperError::NothingToRevisit)
    }

    // ── Echoes ──────────────────────────────────────────────────────────────────────────────

    fn echo_original(&self, id: &str) -> Result<EchoOriginal> {
        let generation = self.store().generation(id)?.ok_or(AutoPaperError::NotFound)?.generation;
        if generation.status != GenerationStatus::Ok {
            return Err(invalid(InvalidInputReason::NothingToEcho, "only finished wallpapers can be echoed"));
        }
        Ok(EchoOriginal { id: generation.id, created_at: generation.created_at, concept: generation.concept })
    }

    /// Originals eligible after `quiet_days` (`echo::eligible`), without ones cleared from history.
    fn eligible_echoes(&self, now: i64, quiet_days: i64) -> Result<Vec<MemoryRow>> {
        let (memory, hidden): (Vec<MemoryRow>, HashSet<String>) = {
            let store = self.store();
            (store.memory()?, store.hidden_ids()?)
        };
        Ok(echo::eligible(&memory, now, quiet_days)
            .into_iter()
            .filter(|row| !hidden.contains(&row.id))
            .cloned()
            .collect())
    }

    /// One of `eligible`, weighted by `echo::pick` (the active mood's originals first).
    fn pick_echo(&self, eligible: &[MemoryRow], now: i64) -> Option<String> {
        let rows: Vec<&MemoryRow> = eligible.iter().collect();
        let mood = self.store().active_mood_id().ok();
        echo::pick(&rows, now, mood.as_deref(), &mut *lock(&self.rng)).map(|row| row.id.clone())
    }

    /// By chance (`settings.echoes`), when an original is eligible: the original a scheduled wallpaper echoes.
    fn scheduled_echo_original(&self, settings: &Settings, now: i64) -> Result<Option<EchoOriginal>> {
        let chance = settings.echoes.chance();
        if chance <= 0.0 {
            return Ok(None);
        }
        let eligible = self.eligible_echoes(now, settings.quiet_period.days())?;
        if eligible.is_empty() || lock(&self.rng).random::<f64>() >= chance {
            return Ok(None);
        }
        self.pick_echo(&eligible, now).map(|id| self.echo_original(&id)).transpose()
    }

    /// For `generate(EchoRequest)`: one past its quiet period if any, else any finished wallpaper that
    /// isn't disliked or cleared (an explicit request ignores the quiet period and the lineage rule).
    fn any_echo_original(&self) -> Result<EchoOriginal> {
        let now = self.now();
        let quiet_days = self.store().settings()?.quiet_period.days();
        let mut eligible = self.eligible_echoes(now, quiet_days)?;
        if eligible.is_empty() {
            let (memory, hidden) = {
                let store = self.store();
                (store.memory()?, store.hidden_ids()?)
            };
            eligible =
                memory.into_iter().filter(|row| echo::rating_allows(row.rating) && !hidden.contains(&row.id)).collect();
        }
        let id = self
            .pick_echo(&eligible, now)
            .ok_or_else(|| invalid(InvalidInputReason::NothingToEcho, "there's nothing to echo yet"))?;
        self.echo_original(&id)
    }

    // ── Making ──────────────────────────────────────────────────────────────────────────────

    /// `make`, saying when it failed whether how painting is set up must change first (`Failed::needs_setup`).
    async fn attempt(
        self: &Arc<Self>,
        trigger: Trigger,
        echo: Option<EchoOriginal>,
        observer: Option<&Arc<dyn ProgressObserver>>,
    ) -> std::result::Result<Generation, Failed> {
        let (mut job, text, image) = self.prepare(trigger, echo)?;
        self.stage(observer, ProgressStage::CheckingServices);
        if let Err(error) = self.check_services(&job, text.as_ref(), image.as_ref()).await {
            let service_mood = if matches!(error, AutoPaperError::Cancelled) { None } else { Some(job.mood_id.clone()) };
            return Err(Failed { error, needs_setup: false, service_mood });
        }
        // Availability is checked for both roles even when the chosen local workflow needs fixing.
        // Unsupported workflows still stop before paid work and keep their normal setup-error behavior.
        image.check_model(&job.settings.image_provider.model)?;
        match self.run(&mut job, text.as_ref(), image.as_ref(), observer).await {
            Ok(generation) => {
                // Every new wallpaper restarts the schedule, whatever asked for it ("New Wallpaper Now" too); a
                // scheduled one then notes how early it started (`run_due`).
                if let Err(error) = self.fill_slot(generation.created_at, generation.created_at) {
                    tracing::warn!(%error, "couldn't restart the schedule");
                }
                Ok(generation)
            }
            Err(error) => {
                self.record_failure(&job, &error);
                Err(Failed { error, needs_setup: job.needs_setup, service_mood: None })
            }
        }
    }

    /// Settings, keywords, providers, the size to request and the budget check. No provider is contacted
    /// when this fails; the Console still records the block and its original settings.
    fn prepare(&self, trigger: Trigger, echo: Option<EchoOriginal>) -> Result<Prepared> {
        let started = self.now();
        let (settings, keywords, mood) = {
            let store = self.store();
            (store.settings()?, store.keywords()?, store.active_mood()?)
        };
        let text = match settings.text_provider.kind {
            ProviderKind::Demo => self.text_provider(&settings.text_provider, &settings)?,
            _ => registry::text_provider(&settings.text_provider, &self.run_deps(&settings, &settings.text_provider, "Writing"))?,
        };
        let image = match settings.image_provider.kind {
            ProviderKind::Demo => self.image_provider(&settings.image_provider, &settings)?,
            _ => registry::image_provider(&settings.image_provider, &self.run_deps(&settings, &settings.image_provider, "Painting"))?,
        };
        let text_model = model_or(&settings.text_provider.model, text.default_model()).to_string();
        let image_model = model_or(&settings.image_provider.model, image.default_model()).to_string();
        let (width, height) = self.request_size(image.as_ref(), &image_model);
        let image_steps = image.steps(&settings.image_provider.model);
        let estimate = pricing::text_cost(text.kind(), &text_model, &pricing::TYPICAL_COMPOSE_USAGE)
            .saturating_add(image_price(image.kind(), &image_model, width, height, settings.image_quality));
        if !self.budget_allows(&settings, started, estimate)? {
            return Err(budget_reached(&settings));
        }
        let echo = echo.map(|original| {
            let axes = echo::choose_axes(settings.surprise, &original.concept, &mut *lock(&self.rng));
            let age = echo::age_text(started.saturating_sub(original.created_at));
            EchoJob { original, axes, age }
        });
        let job = Job {
            trigger,
            started,
            text_origin: origin_of(&settings.text_provider),
            image_origin: origin_of(&settings.image_provider),
            settings,
            keywords,
            mood_id: mood.id,
            mood_name: mood.name,
            echo,
            text_kind: text.kind(),
            text_model,
            image_steps,
            image_kind: image.kind(),
            image_model,
            width,
            height,
            spent: 0,
            images: 0,
            answers_read: 0,
            problems: Vec::new(),
            chosen: None,
            needs_setup: false,
        };
        Ok((job, text, image))
    }

    /// Both roles finish their check even when one fails. Only cheap read-only requests are sent.
    async fn check_services(&self, job: &Job, text: &dyn TextProvider, image: &dyn ImageProvider) -> Result<()> {
        self.check_cancel()?;
        let check = async {
            let writing = async {
                let result = async {
                    text.check_ready()?;
                    service_check(text.kind(), text.check_available()).await
                }.await;
                self.record_service_check("Writing", job.text_kind, &job.text_model, &result);
                result
            };
            let painting = async {
                let result = async {
                    image.check_ready()?;
                    service_check(image.kind(), image.check_available()).await
                }.await;
                self.record_service_check("Painting", job.image_kind, &job.image_model, &result);
                result
            };
            let (writing, painting) = tokio::join!(writing, painting);
            writing.and(painting)
        };
        self.unless_cancelled(check).await
    }

    fn record_service_check(&self, role: &str, provider: ProviderKind, model: &str, result: &Result<()>) {
        let outcome = match result { Ok(()) => "available".into(), Err(error) => error.to_string() };
        self.console.event(None, "Checking services", Some(provider), model, "service_check", &format!("{role}: {outcome}"));
    }

    async fn service_fallback(self: &Arc<Self>, mood: String, error: AutoPaperError) -> Result<Shown> {
        if let Some(run) = lock(&self.console.active).as_mut() {
            run.status = RunStatus::Failed;
            run.detail = self.console.text(&error.to_string());
        }
        let inner = self.clone();
        let selected = tokio::task::spawn_blocking(move || inner.latest_usable_in_mood(&mood)).await
            .map_err(|error| AutoPaperError::Storage { detail: error.to_string() })??;
        self.check_cancel()?;
        match selected {
            Some(generation) => {
                self.console.event(None, "Fallback", None, "", "fallback", &format!(
                    "Showing the latest usable wallpaper from the selected mood: “{}” ({}). No new wallpaper was generated.",
                    generation.concept.title, generation.id));
                Ok(Shown { generation, revisit: Some(RevisitReason::ServicesUnavailable) })
            }
            None => {
                self.console.event(None, "Fallback", None, "", "fallback", "No usable wallpaper remains in the selected mood. The current wallpaper stays in place.");
                Err(error)
            }
        }
    }

    fn latest_usable_in_mood(&self, mood: &str) -> Result<Option<Generation>> {
        let limits = DecodeLimits::default();
        let mut offset = 0;
        loop {
            let page = self.store().history_by_mood(HistoryFilter::All, Some(mood), 50, offset)?;
            let count = page.len();
            for generation in page {
                if generation.rating == Rating::Disliked { continue; }
                let Some(path) = &generation.image_path else { continue };
                let Ok(metadata) = fs::metadata(path) else { continue };
                if !metadata.is_file() || metadata.len() > limits.max_bytes as u64 { continue; }
                let Ok(bytes) = fs::read(path) else { continue };
                if imaging::decode(&bytes, &limits).is_ok() { return Ok(Some(generation)); }
            }
            if count < 50 { return Ok(None); }
            offset += count as u32;
        }
    }

    async fn run(
        self: &Arc<Self>,
        job: &mut Job,
        text: &dyn TextProvider,
        image: &dyn ImageProvider,
        observer: Option<&Arc<dyn ProgressObserver>>,
    ) -> Result<Generation> {
        let mut gentler = false;
        let response = loop {
            let chosen = match self.choose(job, text, gentler, observer).await {
                Err(AutoPaperError::Refused { .. }) if !gentler => {
                    gentler = true;
                    self.stat(|stats| stats.refusal_recomposes += 1);
                    continue;
                }
                other => other?,
            };
            let prompt = chosen.composed.concept.prompt.clone();
            self.console.event(None, "Selection", Some(job.text_kind), &chosen.model, "selection",
                &format!("Selected “{}”\nNovelty penalty: {:.4}; score: {:.4}; least-similar fallback: {}\n{}",
                    chosen.composed.concept.title, chosen.report.max_penalty, chosen.score, chosen.least_similar, prompt));
            job.chosen = Some(chosen);
            self.check_cancel()?;
            self.stage(observer, ProgressStage::Generating);
            let expected = self
                .estimate_secs(ProviderJob::Images, job.image_kind, &job.image_origin, &job.image_model, job.width, job.height, job.image_steps)
                .unwrap_or_else(|error| {
                    tracing::warn!(%error, "couldn't read how long paintings took");
                    None
                });
            let request = ImageRequest {
                model: job.settings.image_provider.model.trim().to_string(),
                prompt,
                width: job.width,
                height: job.height,
                quality: job.settings.image_quality,
                seed: Some(self.next_u64()),
                progress: None,
                expected_secs: expected,
            };
            self.console.event(None, "Painting", Some(job.image_kind), &job.image_model, "parameters",
                &format!("{} × {} · {:?} · seed {:?}\n{}", request.width, request.height, request.quality, request.seed, request.prompt));
            let started = tokio::time::Instant::now();
            match self.paint(image, request, job.image_steps.is_some()).await {
                Ok(response) => {
                    self.record_timing(Timing {
                        job: ProviderJob::Images,
                        provider: job.image_kind,
                        origin: job.image_origin.clone(),
                        model: job.image_model.clone(),
                        width: job.width,
                        height: job.height,
                        steps: job.image_steps,
                        seconds: started.elapsed().as_secs_f64(),
                        finished_at: self.now(),
                    }, &response.model);
                    break response;
                }
                Err(AutoPaperError::Refused { .. }) if !gentler => {
                    gentler = true;
                    self.stat(|stats| stats.refusal_recomposes += 1);
                }
                Err(error) => {
                    job.needs_setup = needs_setup(job.image_kind, &error);
                    return Err(error);
                }
            }
        };

        job.image_model = answered_model(&response.model, &job.image_model);
        let cost = response.reported_cost_microusd.unwrap_or_else(|| {
            image_price(job.image_kind, &job.image_model, job.width, job.height, job.settings.image_quality)
        });
        job.spent = job.spent.saturating_add(cost);
        job.images = 1;
        self.record_spend(job, cost, 1);
        self.run_cost(job);

        self.check_cancel()?;
        self.stage(observer, ProgressStage::Downloading);
        let bytes = response.bytes;
        let (format, picture, bytes) = blocking(move || {
            let (format, picture) = imaging::decode(&bytes, &DecodeLimits::default())?;
            Ok((format, picture, bytes))
        })
        .await?;

        self.check_cancel()?;
        self.stage(observer, ProgressStage::Rendering);
        let id = Uuid::now_v7().to_string();
        let image_path = self.dirs.images.join(year_of(job.started)).join(format!("{id}.{}", format.extension()));
        let thumb_path = self.dirs.thumbs.join(format!("{id}.jpg"));
        let ((width, height), phash) = blocking({
            let (image_path, thumb_path) = (image_path.clone(), thumb_path.clone());
            move || {
                imaging::write_original(&bytes, &image_path)?;
                if let Err(error) = imaging::write_thumbnail(&picture, &thumb_path) {
                    let _ = fs::remove_file(&image_path);
                    return Err(error);
                }
                Ok((picture.dimensions(), imaging::phash(&picture)))
            }
        })
        .await?;

        let chosen = job.chosen.clone().ok_or_else(|| internal("no concept was chosen"))?;
        let echo_note = job.echo.as_ref().map(|echo| {
            chosen
                .composed
                .echo_note
                .clone()
                .filter(|note| !note.trim().is_empty())
                .unwrap_or_else(|| format!("Echo of “{}” ({})", echo.original.concept.title.trim(), echo.age))
        });
        let generation = Generation {
            id: id.clone(),
            created_at: job.started,
            trigger: job.trigger,
            status: GenerationStatus::Ok,
            concept: chosen.composed.concept.clone(),
            image_path: Some(path_string(&image_path)),
            thumb_path: Some(path_string(&thumb_path)),
            width,
            height,
            rating: Rating::Unrated,
            echo_of: job.echo.as_ref().map(|echo| echo.original.id.clone()),
            echo_note,
            text_provider: job.text_kind,
            text_model: chosen.model.clone(),
            image_provider: job.image_kind,
            image_model: job.image_model.clone(),
            surprise: job.settings.surprise,
            keywords: snapshot(&job.keywords),
            cost_microusd: job.spent,
            last_shown_at: None,
            shown_count: 0,
            error: None,
            mood_id: Some(job.mood_id.clone()),
            mood_name: Some(job.mood_name.clone()),
        };
        let stored = StoredGeneration {
            generation: generation.clone(),
            embedding: chosen.embedding,
            embedding_model: self.embedder.model_id().to_string(),
            phash: Some(phash),
            least_similar: chosen.least_similar,
        };
        let inserted = self.store().insert_generation(&stored);
        if let Err(error) = inserted {
            let _ = fs::remove_file(&image_path);
            let _ = fs::remove_file(&thumb_path);
            return Err(error);
        }
        // From here the wallpaper exists: bookkeeping problems are logged, not returned.
        if let Err(error) = self.record_success() {
            tracing::warn!(%error, "couldn't reset the failure backoff");
        }
        self.log_near_duplicates(&id, phash);
        // Holding `making`, no other generation is between writing its files and storing its row.
        if let Err(error) = self.remove_orphans() {
            tracing::warn!(%error, "couldn't remove image files no wallpaper owns");
        }
        if let Err(error) = self.forget_old_failures() {
            tracing::warn!(%error, "couldn't delete old failed attempts");
        }
        // The new wallpaper is kept: the host hasn't shown it yet.
        if let Err(error) = self.prune_files(Some(&id)) {
            tracing::warn!(%error, "couldn't prune images to the storage limit");
        }
        self.stage(observer, ProgressStage::Done);
        Ok(generation)
    }

    /// The concept to paint. A scheduled echo that yields no usable echo becomes a new wallpaper; anything
    /// else with no usable candidate is `InvalidResponse`.
    async fn choose(
        self: &Arc<Self>,
        job: &mut Job,
        text: &dyn TextProvider,
        gentler: bool,
        observer: Option<&Arc<dyn ProgressObserver>>,
    ) -> Result<Evaluated> {
        if let Some(chosen) = self.compose(job, text, gentler, observer).await? {
            return Ok(chosen);
        }
        if job.echo.is_some() && job.trigger == Trigger::Scheduled {
            tracing::info!("no candidate made a recognisable echo; composing a new wallpaper instead");
            job.echo = None;
            let chosen = self.compose(job, text, gentler, observer).await?;
            return chosen.ok_or_else(|| no_usable(job, false));
        }
        Err(no_usable(job, job.echo.is_some()))
    }

    /// The compose loop: the best valid novel candidate, or after the retries the least similar valid one
    /// (marked `least_similar`), or `None` when no candidate was valid. While nothing valid is novel it asks
    /// again, at most `MAX_COMPOSE_RETRIES` times, naming what the candidates came too close to — but once
    /// there is a valid candidate to fall back on, a retry that doesn't lower its penalty ends the retries:
    /// the keywords leave no room, and asking again only costs. While no answer has had a valid candidate at
    /// all, retries go on as before (unreadable answers, missed Musts).
    async fn compose(
        self: &Arc<Self>,
        job: &mut Job,
        text: &dyn TextProvider,
        gentler: bool,
        observer: Option<&Arc<dyn ProgressObserver>>,
    ) -> Result<Option<Evaluated>> {
        self.ensure_memory_embedded().await?;
        let basis = Arc::new(self.basis(job)?);
        let mut context = self.compose_context(job, gentler, &basis.taste)?;
        let mut least: Option<Evaluated> = None;
        for attempt in 0..=MAX_COMPOSE_RETRIES {
            let candidates = self.compose_once(job, text, &context, &basis, observer).await?;
            let unusable: Vec<Problem> =
                candidates.iter().filter(|candidate| !candidate.valid).flat_map(|c| c.problems.iter().cloned()).collect();
            if !unusable.is_empty() {
                let said: Vec<String> = unusable.iter().map(problem_text).collect();
                tracing::warn!(problems = ?said, "the text model's candidates couldn't be used");
            }
            job.problems.extend(unusable);
            let too_similar = candidates.iter().filter(|candidate| candidate.valid && !candidate.novel).count();
            self.stat(|stats| {
                stats.too_similar_candidates =
                    stats.too_similar_candidates.saturating_add(u32::try_from(too_similar).unwrap_or(u32::MAX));
            });
            if let Some(best) = best_novel(&candidates) {
                return Ok(Some(candidates[best].clone()));
            }
            let had_fallback = least.is_some();
            let mut closer = false;
            if let Some(index) = least_similar(&candidates)
                && least.as_ref().is_none_or(|kept| candidates[index].report.max_penalty < kept.report.max_penalty)
            {
                least = Some(candidates[index].clone());
                closer = true;
            }
            if attempt == MAX_COMPOSE_RETRIES {
                break;
            }
            if had_fallback && !closer {
                tracing::info!("asking again came no closer to a new idea; taking the least similar one");
                self.stat(|stats| stats.retries_stopped += 1);
                break;
            }
            if candidates.iter().any(|candidate| candidate.valid) {
                self.stat(|stats| stats.novelty_retries += 1);
            } else {
                self.stat(|stats| stats.invalid_retries += 1);
            }
            add_too_close(&candidates, &basis, &mut context.too_close);
            context.corrections = most_common(&job.problems).into_iter().map(composer::correction).collect();
        }
        Ok(least.map(|mut fallback| {
            self.stat(|stats| stats.least_similar_fallbacks += 1);
            fallback.least_similar = true;
            fallback
        }))
    }

    fn basis(&self, job: &Job) -> Result<Basis> {
        let (memory, taste_rows) = {
            let store = self.store();
            (store.memory()?, store.taste_rows()?)
        };
        let calibration = Calibration::for_model(self.embedder.model_id());
        Ok(Basis {
            memory,
            taste: Taste::from_rows(taste_rows),
            policy: NoveltyPolicy { quiet_days: job.settings.quiet_period.days(), threshold: calibration.threshold },
            calibration,
            now: job.started,
            surprise: job.settings.surprise,
        })
    }

    fn compose_context(&self, job: &Job, gentler: bool, learned: &Taste) -> Result<ComposeContext> {
        let (liked, disliked) = learned.hints(job.started);
        let (liked, disliked) = if liked.is_empty() && disliked.is_empty() {
            (liked, disliked)
        } else {
            let labels = self.taste_labels()?;
            (in_words(liked, &labels), in_words(disliked, &labels))
        };
        let recent = self.store().recent_summaries(u32::try_from(composer::MAX_RECENT).unwrap_or(u32::MAX))?;
        Ok(ComposeContext {
            keywords: job.keywords.clone(),
            surprise: job.settings.surprise,
            locale: self.config.locale.clone(),
            liked,
            disliked,
            recent,
            too_close: Vec::new(),
            corrections: Vec::new(),
            echo: job.echo.as_ref().map(|echo| EchoBrief {
                original: echo.original.concept.clone(),
                age: echo.age.clone(),
                axes: echo.axes.clone(),
            }),
            gentler,
            appearance: if job.settings.match_system_theme { *lock(&self.appearance) } else { None },
        })
    }

    /// One call to the text model, its candidates assessed. An unreadable answer gives no candidates
    /// (providers pass an answer they couldn't read as JSON on as text, with its usage, so it is paid for
    /// and asked again).
    async fn compose_once(
        self: &Arc<Self>,
        job: &mut Job,
        text: &dyn TextProvider,
        context: &ComposeContext,
        basis: &Arc<Basis>,
        observer: Option<&Arc<dyn ProgressObserver>>,
    ) -> Result<Vec<Evaluated>> {
        self.check_cancel()?;
        self.stage(observer, ProgressStage::Composing);
        let mut request = composer::build_request(context, job.settings.text_provider.model.trim());
        // How long this writer has taken here: a local server waits longer than 10 minutes only when it usually needs it.
        request.expected_secs = self
            .estimate_secs(ProviderJob::Concepts, job.text_kind, &job.text_origin, &job.text_model, 0, 0, None)
            .unwrap_or_else(|error| {
                tracing::warn!(%error, "couldn't read how long writing took");
                None
            });
        self.console.event(None, "Writing", Some(job.text_kind), &job.text_model, "instructions",
            &format!("System instructions:\n{}\n\nUser prompt:\n{}\n\nSchema:\n{}\n\nTemperature (where supported): {}", request.system, request.user, request.schema, request.temperature));
        let started = tokio::time::Instant::now();
        let response = self.unless_cancelled(text.compose(request)).await?;
        self.record_timing(Timing {
            job: ProviderJob::Concepts,
            provider: job.text_kind,
            origin: job.text_origin.clone(),
            model: job.text_model.clone(),
            width: 0,
            height: 0,
            steps: None,
            seconds: started.elapsed().as_secs_f64(),
            finished_at: self.now(),
        }, &response.model);
        let model = answered_model(&response.model, &job.text_model);
        job.text_model = model.clone();
        let cost = pricing::text_cost(job.text_kind, &model, &response.usage);
        job.spent = job.spent.saturating_add(cost);
        self.record_spend(job, cost, 0);
        self.run_cost(job);
        self.console.event(None, "Writing", Some(job.text_kind), &model, "candidates",
            &format!("Answering model: {model}\nInput tokens: {}; output tokens: {}\n{}", response.usage.input_tokens, response.usage.output_tokens, response.output));
        self.stat(|stats| stats.compose_calls += 1);

        self.check_cancel()?;
        self.stage(observer, ProgressStage::CheckingMemory);
        let candidates = match composer::parse(&response.output, context.echo.is_some()) {
            Ok(candidates) => candidates,
            Err(error) => {
                self.console.event(None, "CheckingMemory", Some(job.text_kind), &model, "rejected", &error.to_string());
                tracing::warn!(error = %net::redact(&error.to_string()), "the text model's answer couldn't be read");
                return Ok(Vec::new());
            }
        };
        job.answers_read = job.answers_read.saturating_add(1);
        let jitters: Vec<f32> = {
            let mut rng = lock(&self.rng);
            candidates.iter().map(|_| rng.random::<f32>()).collect()
        };
        let inner = self.clone();
        let basis = basis.clone();
        let keywords = job.keywords.clone();
        let echo = job.echo.clone();
        let model_for_trace = model.clone();
        let evaluated = blocking(move || inner.evaluate(candidates, &keywords, &basis, echo.as_ref(), &jitters, &model)).await?;
        for (index, candidate) in evaluated.iter().enumerate() {
            self.console.event(None, "CheckingMemory", Some(job.text_kind), &model_for_trace, "evaluation",
                &format!("Candidate {}: “{}”\nValid: {}; novel: {}; similarity penalty: {:.4}; score: {:.4}\nKeyword checks: {}\nEcho in band: {:?}; echo changed: {:?}",
                    index + 1, candidate.composed.concept.title, candidate.valid, candidate.novel, candidate.report.max_penalty, candidate.score,
                    if candidate.problems.is_empty() { "Passed".into() } else { candidate.problems.iter().map(problem_text).collect::<Vec<_>>().join("; ") }, candidate.echo_in_band, candidate.echo_changed));
        }
        Ok(evaluated)
    }

    /// Checks, embeds, assesses and scores candidates (blocking: runs the embedding model).
    fn evaluate(
        &self,
        candidates: Vec<Composed>,
        keywords: &[Keyword],
        basis: &Basis,
        echo: Option<&EchoJob>,
        jitters: &[f32],
        model: &str,
    ) -> Result<Vec<Evaluated>> {
        let model_id = self.embedder.model_id();
        let original_embedding =
            echo.map(|echo| self.embedder.embed(&embed::concept_text(&echo.original.concept))).transpose()?;
        candidates
            .into_iter()
            .zip(jitters)
            .map(|(composed, &jitter)| {
                let problems = composer::check(&composed.concept, keywords, echo.is_some());
                let embedding = self.embedder.embed(&embed::concept_text(&composed.concept))?;
                let lineage = echo.map(|echo| echo.original.id.as_str());
                let report = novelty::assess(&embedding, model_id, &basis.memory, basis.now, &basis.policy, lineage);
                let echo_cosine = original_embedding.as_ref().map(|original| embed::cosine(&embedding, original));
                let echo_in_band = echo_cosine.map(|cosine| basis.calibration.in_echo_band(cosine));
                let echo_changed =
                    echo.map(|echo| changed_along_axes(&echo.original.concept, &composed.concept, &echo.axes));
                let taste = basis.taste.score(&composed.concept, basis.now);
                let score = composer::score(report.score(), taste, basis.surprise, jitter);
                let valid = problems.is_empty() && echo_in_band.unwrap_or(true) && echo_changed.unwrap_or(true);
                let novel = report.is_novel(&basis.policy);
                Ok(Evaluated {
                    composed,
                    problems,
                    embedding,
                    report,
                    echo_cosine,
                    echo_in_band,
                    echo_changed,
                    taste,
                    score,
                    valid,
                    novel,
                    model: model.to_string(),
                    least_similar: false,
                })
            })
            .collect()
    }

    /// Taste feature keys → the phrase the most recent rated wallpaper with that feature used. Wallpapers
    /// cleared from history count too: their taste is kept, so their words are.
    fn taste_labels(&self) -> Result<HashMap<String, String>> {
        let rated = {
            let store = self.store();
            let mut rated = store.rated_concepts(Rating::Liked, TASTE_LABEL_SOURCES)?;
            rated.extend(store.rated_concepts(Rating::Disliked, TASTE_LABEL_SOURCES)?);
            rated
        };
        let mut labels = HashMap::new();
        // Newest first within each list; liked before disliked. The first phrase seen for a key wins.
        for concept in &rated {
            for (key, phrase) in taste::labelled_features(concept) {
                labels.entry(key).or_insert(phrase);
            }
        }
        Ok(labels)
    }

    /// Re-embeds remembered generations whose embedding came from another model, once per engine, before
    /// the first novelty check.
    async fn ensure_memory_embedded(self: &Arc<Self>) -> Result<()> {
        if self.memory_current.load(Ordering::SeqCst) {
            return Ok(());
        }
        let inner = self.clone();
        let count = blocking(move || inner.reembed()).await?;
        if count > 0 {
            tracing::info!(count, model = self.embedder.model_id(), "re-embedded memory for the current model");
        }
        self.stat(|stats| stats.reembedded = stats.reembedded.saturating_add(count));
        self.memory_current.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn reembed(&self) -> Result<u32> {
        let model = self.embedder.model_id().to_string();
        let stale: Vec<String> =
            self.store().memory()?.into_iter().filter(|row| row.embedding_model != model).map(|row| row.id).collect();
        let mut count = 0u32;
        for id in stale {
            let Some(stored) = self.store().generation(&id)? else { continue };
            let vector = self.embedder.embed(&embed::concept_text(&stored.generation.concept))?;
            self.store().update_embedding(&id, &vector, &model)?;
            count = count.saturating_add(1);
        }
        Ok(count)
    }

    /// Adds one paid call to the month's spend, right as it answers (logged if it can't be stored).
    fn record_spend(&self, job: &Job, microusd: u64, images: u32) {
        if microusd == 0 && images == 0 {
            return;
        }
        let added = self.store().add_spend(&schedule::month_key(job.started), microusd, images);
        if let Err(error) = added {
            tracing::warn!(%error, "couldn't record the spend");
        }
    }

    /// Stores a failed or refused row (except when the person cancelled, or nothing could be attempted: a
    /// missing key, the budget). What the attempt cost is already in the month's spend.
    fn record_failure(&self, job: &Job, error: &AutoPaperError) {
        if matches!(
            error,
            AutoPaperError::Cancelled | AutoPaperError::MissingKey { .. } | AutoPaperError::BudgetReached { .. }
        ) {
            return;
        }
        let status = match error {
            AutoPaperError::Refused { .. } => GenerationStatus::Refused,
            _ => GenerationStatus::Failed,
        };
        let detail = net::redact(&error.to_string());
        self.run_cost(job);
        tracing::warn!(?status, error = %detail, "a wallpaper couldn't be made");
        let generation = Generation {
            id: Uuid::now_v7().to_string(),
            created_at: job.started,
            trigger: job.trigger,
            status,
            concept: job.chosen.as_ref().map(|chosen| chosen.composed.concept.clone()).unwrap_or_default(),
            image_path: None,
            thumb_path: None,
            width: 0,
            height: 0,
            rating: Rating::Unrated,
            echo_of: job.echo.as_ref().map(|echo| echo.original.id.clone()),
            echo_note: None,
            text_provider: job.text_kind,
            text_model: job.chosen.as_ref().map_or_else(|| job.text_model.clone(), |chosen| chosen.model.clone()),
            image_provider: job.image_kind,
            image_model: job.image_model.clone(),
            surprise: job.settings.surprise,
            keywords: snapshot(&job.keywords),
            cost_microusd: job.spent,
            last_shown_at: None,
            shown_count: 0,
            error: Some(detail),
            mood_id: Some(job.mood_id.clone()),
            mood_name: Some(job.mood_name.clone()),
        };
        let stored = StoredGeneration {
            generation,
            embedding: Vec::new(),
            embedding_model: String::new(),
            phash: None,
            least_similar: job.chosen.as_ref().is_some_and(|chosen| chosen.least_similar),
        };
        let inserted = self.store().insert_generation(&stored);
        if let Err(store_error) = inserted {
            tracing::warn!(error = %store_error, "couldn't record a failed attempt");
        }
    }

    /// Logs (doesn't regenerate — that would bill twice) when the new image looks like a recent one.
    fn log_near_duplicates(&self, id: &str, phash: u64) {
        let store = self.store();
        let Ok(recent) = store.history(HistoryFilter::All, RECENT_IMAGES_COMPARED, 0) else { return };
        for other in recent.iter().filter(|generation| generation.id != id) {
            if let Ok(Some(stored)) = store.generation(&other.id)
                && let Some(hash) = stored.phash
                && imaging::hamming(hash, phash) <= NEAR_DUPLICATE_BITS
            {
                tracing::info!(id, near = %other.id, "the new image looks like a recent one (perceptual hash)");
            }
        }
    }

    // ── Files ───────────────────────────────────────────────────────────────────────────────

    /// The generation whose renders are on the desktop: the one most recently marked shown (even when it has
    /// been cleared from history or deleted since), else `current()` for a database from before that was
    /// recorded.
    fn desktop_id(&self) -> Result<Option<String>> {
        let store = self.store();
        if let Some(Value::String(id)) = store.state_get(STATE_DESKTOP)? {
            return Ok(Some(id));
        }
        Ok(store.current()?.map(|generation| generation.id))
    }

    fn usage(&self) -> Result<StorageUsage> {
        let (image_bytes, images_on_disk) = dir_usage(&self.dirs.images);
        let (thumb_bytes, _) = dir_usage(&self.dirs.thumbs);
        let (render_bytes, _) = dir_usage(&self.dirs.renders);
        Ok(StorageUsage {
            image_bytes: image_bytes.saturating_add(thumb_bytes).saturating_add(render_bytes),
            generations: self.store().generation_count()?,
            images_on_disk,
        })
    }

    /// See `Engine::prune`; `keep` is a generation just made (the host hasn't shown it yet). Returns how
    /// many originals were deleted.
    fn prune_files(&self, keep: Option<&str>) -> Result<u32> {
        let limit = u64::from(self.store().settings()?.storage_limit_mb) * 1024 * 1024;
        let mut used = self.usage()?.image_bytes;
        if used <= limit {
            return Ok(0);
        }
        let current = self.store().current()?.map(|generation| generation.id);
        let desktop = self.desktop_id()?;
        let protected =
            |id: &str| current.as_deref() == Some(id) || desktop.as_deref() == Some(id) || keep == Some(id);
        used = used.saturating_sub(self.remove_renders(|id| !protected(id)));
        let mut pruned = 0;
        // Bound first: a guard in the loop header would live through the loop and deadlock the body.
        let candidates = self.store().prunable()?;
        for generation in candidates {
            if used <= limit {
                break;
            }
            if protected(&generation.id) {
                continue;
            }
            // A file that is still there (another app has it open) stays tracked for the next prune.
            let Some(freed) = generation.image_path.as_deref().map_or(Some(0), remove_file) else { continue };
            self.store().set_image_paths(&generation.id, None, generation.thumb_path.as_deref())?;
            used = used.saturating_sub(freed);
            pruned += 1;
        }
        if used > limit {
            tracing::warn!(used, limit, "liked and current wallpapers alone pass the storage limit");
        }
        Ok(pruned)
    }

    /// Deletes the renders of generations `which` selects; returns the bytes freed.
    fn remove_renders(&self, which: impl Fn(&str) -> bool) -> u64 {
        let Ok(entries) = fs::read_dir(&self.dirs.renders) else { return 0 };
        let mut freed = 0;
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            // "<id>-<w>x<h>.jpg": the id is a UUID, so its own hyphens come before the last one.
            let Some((id, _)) = name.rsplit_once('-') else { continue };
            if which(id) {
                freed += remove_file(&entry.path().to_string_lossy()).unwrap_or(0);
            }
        }
        freed
    }

    /// Deletes a generation's files (its renders too, unless `keep_renders`: it's on the desktop); returns the
    /// (image, thumbnail) paths that couldn't be deleted.
    fn remove_files(&self, generation: &Generation, keep_renders: bool) -> (Option<String>, Option<String>) {
        let left = |path: &Option<String>| path.clone().filter(|path| remove_file(path).is_none());
        let left = (left(&generation.image_path), left(&generation.thumb_path));
        if !keep_renders {
            self.remove_renders(|id| id == generation.id);
        }
        left
    }

    /// Deletes failed and refused attempts older than `FAILED_ROWS_KEPT_DAYS`; returns how many.
    fn forget_old_failures(&self) -> Result<u32> {
        let cutoff = self.now().saturating_sub(FAILED_ROWS_KEPT_DAYS * SECONDS_PER_DAY);
        let deleted = self.store().delete_unsuccessful_before(cutoff)?;
        if deleted > 0 {
            tracing::info!(deleted, "deleted failed attempts older than {FAILED_ROWS_KEPT_DAYS} days");
        }
        Ok(deleted)
    }

    /// Deletes originals and thumbnails whose generation doesn't exist: what a generation leaves when the
    /// host drops its call between writing the files and storing the row. Only names the engine writes
    /// (`<uuid>.<ext>`) older than `ORPHAN_MIN_AGE` are touched. Also deletes display renders no generation in
    /// history owns (kept while their wallpaper was on the desktop: deleted, or cleared from history) once
    /// they're no longer on the desktop. Call it only while holding `making`. Returns the bytes freed.
    fn remove_orphans(&self) -> Result<u64> {
        let (ids, hidden) = {
            let store = self.store();
            (store.generation_ids()?, store.hidden_ids()?)
        };
        let desktop = self.desktop_id()?;
        let mut freed = self.remove_renders(|id| {
            let in_history = ids.contains(id) && !hidden.contains(id);
            Uuid::parse_str(id).is_ok() && !in_history && desktop.as_deref() != Some(id)
        });
        let stale = |entry: &fs::DirEntry| {
            let modified = entry.metadata().and_then(|metadata| metadata.modified());
            modified.is_ok_and(|at| at.elapsed().is_ok_and(|age| age >= ORPHAN_MIN_AGE))
        };
        let orphans_in = |dir: &Path| -> Vec<PathBuf> {
            let Ok(entries) = fs::read_dir(dir) else { return Vec::new() };
            entries
                .flatten()
                .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()) && stale(entry))
                .map(|entry| entry.path())
                .filter(|path| {
                    path.file_stem()
                        .and_then(|stem| stem.to_str())
                        .is_some_and(|stem| Uuid::parse_str(stem).is_ok() && !ids.contains(stem))
                })
                .collect()
        };
        let mut orphans = orphans_in(&self.dirs.thumbs);
        if let Ok(years) = fs::read_dir(&self.dirs.images) {
            for year in years.flatten().filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir())) {
                orphans.extend(orphans_in(&year.path()));
            }
        }
        for orphan in orphans {
            tracing::info!(file = %orphan.display(), "removing an image file no wallpaper owns");
            freed += remove_file(&orphan.to_string_lossy()).unwrap_or(0);
        }
        Ok(freed)
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────────────────────────

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Reports a stage. A host observer that fails (UniFFI turns a foreign exception into a panic) is logged and
/// ignored: progress is a courtesy, and the generation's bookkeeping must still run.
async fn service_check(provider: ProviderKind, request: impl std::future::Future<Output = Result<()>>) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(8), request).await.unwrap_or_else(|_| {
        Err(AutoPaperError::ProviderUnavailable { provider, reason: ProviderUnavailableReason::TimedOut,
            detail: "The service availability check did not finish within 8 seconds.".into() })
    })
}

fn progress(observer: Option<&Arc<dyn ProgressObserver>>, stage: ProgressStage) {
    if let Some(observer) = observer
        && catch_unwind(AssertUnwindSafe(|| observer.on_progress(stage))).is_err()
    {
        tracing::warn!(?stage, "the progress observer failed");
    }
}

/// Runs CPU-bound or blocking work on the runtime's blocking pool.
async fn blocking<T, F>(work: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T> + Send + 'static,
{
    tokio::task::spawn_blocking(work).await.map_err(|error| internal(format!("a background task failed: {error}")))?
}

fn invalid(reason: InvalidInputReason, detail: impl Into<String>) -> AutoPaperError {
    AutoPaperError::invalid_input(reason, detail)
}

fn internal(detail: impl Into<String>) -> AutoPaperError {
    AutoPaperError::Internal { detail: detail.into() }
}

fn budget_reached(settings: &Settings) -> AutoPaperError {
    AutoPaperError::BudgetReached { budget_cents: settings.monthly_budget_cents.unwrap_or(0) }
}

/// No usable candidate. When a keyword sank the most candidates (a Must left out, an Avoid brought in), it's
/// `KeywordNotFollowed`, naming it and the mood; otherwise `InvalidResponse`, whose detail names the problem that
/// sank the most, so the log says which rule the model kept breaking.
fn no_usable(job: &Job, echo: bool) -> AutoPaperError {
    let readable = job.answers_read > 0;
    let commonest = most_common(&job.problems).first().copied();
    let keyword = match commonest {
        Some(Problem::MissingMust(keyword)) => Some((keyword, KeywordWeight::Must)),
        Some(Problem::MentionsAvoid(keyword)) => Some((keyword, KeywordWeight::Avoid)),
        _ => None,
    };
    if let Some((keyword, weight)) = keyword
        && readable
    {
        return AutoPaperError::KeywordNotFollowed { keyword: keyword.clone(), weight, mood_id: job.mood_id.clone() };
    }
    let detail: String = if !readable {
        "the text model's answers couldn't be read".into()
    } else if echo {
        "none of the text model's echoes stayed recognisable without repeating the original".into()
    } else {
        "none of the text model's ideas followed the keywords".into()
    };
    let detail = match commonest {
        Some(problem) if readable => format!("{detail}: {}", problem_text(problem)),
        _ => detail,
    };
    AutoPaperError::InvalidResponse { detail }
}

/// Distinct problems, the most frequent first (ties in the order first seen).
fn most_common(problems: &[Problem]) -> Vec<&Problem> {
    let mut counted: Vec<(&Problem, usize)> = Vec::new();
    for problem in problems {
        match counted.iter_mut().find(|(seen, _)| *seen == problem) {
            Some((_, count)) => *count += 1,
            None => counted.push((problem, 1)),
        }
    }
    counted.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    counted.into_iter().map(|(problem, _)| problem).collect()
}

/// The model a provider says answered, when it reads as a model name (no control characters, ≤ 128
/// characters); otherwise the one asked for. It is stored and shown, so nothing a server sends may smuggle
/// terminal escapes into a log or a CLI.
fn answered_model(answered: &str, asked: &str) -> String {
    let answered = answered.trim();
    let plausible = !answered.is_empty() && answered.chars().count() <= 128 && !answered.chars().any(char::is_control);
    if plausible { answered } else { asked }.to_string()
}

/// How a failed scheduled attempt is covered: transient failures, refusals and unusable answers revisit a
/// liked wallpaper; problems the person has to fix don't.
fn revisit_reason(error: &AutoPaperError) -> Option<RevisitReason> {
    match error {
        AutoPaperError::Offline => Some(RevisitReason::Offline),
        AutoPaperError::BudgetReached { .. } => Some(RevisitReason::OverBudget),
        error if error.is_transient() => Some(RevisitReason::ProviderFailed),
        AutoPaperError::Refused { .. } | AutoPaperError::InvalidResponse { .. } => Some(RevisitReason::ProviderFailed),
        _ => None,
    }
}

/// A painting that failed because of how painting is set up, which trying again can't fix: `PaintingFailed` (ComfyUI
/// rejected or couldn't run the workflow — a missing file or node, a node that failed: a model run through another
/// model's graph, out of memory at that size — or has no workflow for the model), or ComfyUI's workflow can't be
/// used or its answers can't be read. ComfyUI's passing problems (not running, too slow, a server error, a job
/// stopped there) are `ProviderUnavailable` and stay worth retrying; so do an unusable answer from a hosted service
/// and anything from the text model.
pub(crate) fn needs_setup(painter: ProviderKind, error: &AutoPaperError) -> bool {
    matches!(error, AutoPaperError::PaintingFailed { .. })
        || painter == ProviderKind::ComfyUi
            && matches!(error, AutoPaperError::InvalidResponse { .. } | AutoPaperError::InvalidInput { .. })
}

/// Where a provider's timings are filed: the server's origin for providers that run on the person's own server
/// (Ollama, ComfyUI, OpenAI-compatible: `http://127.0.0.1:8188`), so another computer's server keeps its own
/// history; empty for hosted services and Demo.
fn origin_of(selection: &ProviderSelection) -> String {
    match selection.kind {
        ProviderKind::Ollama | ProviderKind::ComfyUi | ProviderKind::OpenAiCompatible => {
            let chosen = selection.base_url.as_deref().map(str::trim).filter(|url| !url.is_empty());
            chosen
                .or_else(|| registry::default_base_url(selection.kind))
                .and_then(|base| url::Url::parse(base).ok())
                .map(|url| url.origin().ascii_serialization())
                .unwrap_or_default()
        }
        ProviderKind::OpenAi | ProviderKind::Google | ProviderKind::Demo => String::new(),
    }
}

/// Surprise as stored: 0–1, a non-number → the default.
fn clamped_surprise(surprise: f32) -> f32 {
    if surprise.is_finite() { surprise.clamp(0.0, 1.0) } else { Settings::default().surprise }
}

/// `DEMO_DELAY_VARIABLE` as a duration: whole seconds 1–3600; anything else is ignored (and logged).
fn demo_delay_from_env() -> Option<Duration> {
    let value = std::env::var(DEMO_DELAY_VARIABLE).ok()?;
    parse_demo_delay(&value).or_else(|| {
        tracing::warn!(value, "{DEMO_DELAY_VARIABLE} wants whole seconds from 1 to 3600; ignored");
        None
    })
}

fn parse_demo_delay(value: &str) -> Option<Duration> {
    let seconds: u64 = value.trim().parse().ok()?;
    (1..=demo::MAX_DELAY.as_secs()).contains(&seconds).then(|| Duration::from_secs(seconds))
}

fn model_or<'a>(model: &'a str, default: &'a str) -> &'a str {
    match model.trim() {
        "" => default,
        model => model,
    }
}

/// The price of an image, at the size the provider actually returns (Gemini picks its resolution tier
/// from the quality, whatever size was asked for).
fn image_price(kind: ProviderKind, model: &str, width: u32, height: u32, quality: ImageQuality) -> u64 {
    let (width, height) = match kind {
        ProviderKind::Google => google::output_size(model, width, height, quality).unwrap_or((width, height)),
        _ => (width, height),
    };
    pricing::image_cost(kind, model, width, height, quality)
}

fn validated(mut settings: Settings) -> Result<Settings> {
    settings.surprise = clamped_surprise(settings.surprise);
    settings.storage_limit_mb = settings.storage_limit_mb.max(MIN_STORAGE_LIMIT_MB);
    if !settings.text_provider.kind.writes_concepts() {
        return Err(AutoPaperError::Unsupported {
            provider: settings.text_provider.kind,
            job: "write concepts".into(),
        });
    }
    if !settings.image_provider.kind.makes_images() {
        return Err(AutoPaperError::Unsupported { provider: settings.image_provider.kind, job: "make images".into() });
    }
    for selection in [&mut settings.text_provider, &mut settings.image_provider] {
        selection.model = selection.model.trim().to_string();
        selection.base_url =
            selection.base_url.as_deref().map(str::trim).filter(|url| !url.is_empty()).map(String::from);
        if let Some(url) = &selection.base_url {
            net::check_url(url, &HostPolicy::UserEndpoint)?;
        }
    }
    settings.comfyui_workflow = settings.comfyui_workflow.filter(|workflow| !workflow.trim().is_empty());
    Ok(settings)
}

/// An echo visibly changed when at least one requested axis changed its field (by `feature_key`):
/// time of day, weather, season, style (medium) or composition (viewpoint). Passage of time has no field
/// of its own; with only such axes the check passes.
fn changed_along_axes(original: &Concept, candidate: &Concept, axes: &[EchoAxis]) -> bool {
    let differs = |before: &str, after: &str| {
        let after = feature_key(after);
        !after.is_empty() && feature_key(before) != after
    };
    let mut checkable = false;
    for axis in axes {
        let changed = match axis {
            EchoAxis::TimeOfDay => differs(&original.time_of_day, &candidate.time_of_day),
            EchoAxis::Weather => differs(&original.weather, &candidate.weather),
            EchoAxis::Season => differs(&original.season, &candidate.season),
            EchoAxis::Medium => differs(&original.style, &candidate.style),
            EchoAxis::Viewpoint => differs(&original.composition, &candidate.composition),
            EchoAxis::PassageOfTime => continue,
        };
        checkable = true;
        if changed {
            return true;
        }
    }
    !checkable
}

/// The best valid novel candidate by score.
fn best_novel(candidates: &[Evaluated]) -> Option<usize> {
    candidates
        .iter()
        .enumerate()
        .filter(|(_, candidate)| candidate.valid && candidate.novel)
        .max_by(|(_, a), (_, b)| a.score.total_cmp(&b.score))
        .map(|(index, _)| index)
}

/// The valid candidate least similar to memory.
fn least_similar(candidates: &[Evaluated]) -> Option<usize> {
    candidates
        .iter()
        .enumerate()
        .filter(|(_, candidate)| candidate.valid)
        .min_by(|(_, a), (_, b)| a.report.max_penalty.total_cmp(&b.report.max_penalty))
        .map(|(index, _)| index)
}

/// Adds the summaries of remembered wallpapers the candidates came too close to (closest first), for the
/// retry's "too close" list.
fn add_too_close(candidates: &[Evaluated], basis: &Basis, too_close: &mut Vec<String>) {
    let mut near: Vec<(&str, f32)> = candidates
        .iter()
        .flat_map(|candidate| candidate.report.nearest.iter())
        .filter(|(_, penalty)| *penalty >= basis.policy.threshold)
        .map(|(id, penalty)| (id.as_str(), *penalty))
        .collect();
    near.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    for (id, _) in near {
        if too_close.len() >= MAX_TOO_CLOSE {
            break;
        }
        if let Some(row) = basis.memory.iter().find(|row| row.id == id) {
            let summary = row.summary.trim();
            if !summary.is_empty() && !too_close.iter().any(|kept| kept == summary) {
                too_close.push(summary.to_string());
            }
        }
    }
}

fn preview(candidate: &Evaluated, memory: &[MemoryRow]) -> CandidatePreview {
    let nearest = candidate
        .report
        .nearest
        .first()
        .and_then(|(id, _)| memory.iter().find(|row| &row.id == id))
        .map(|row| row.title.clone());
    let mut problems: Vec<String> = candidate.problems.iter().map(problem_text).collect();
    if let (Some(cosine), Some(false)) = (candidate.echo_cosine, candidate.echo_in_band) {
        problems.push(format!("its similarity to the original ({cosine:.3}) is outside the echo band"));
    }
    if candidate.echo_changed == Some(false) {
        problems.push("none of the requested echo changes is visible in its fields".to_string());
    }
    CandidatePreview {
        concept: candidate.composed.concept.clone(),
        echo_note: candidate.composed.echo_note.clone(),
        problems,
        similarity: candidate.report.max_penalty,
        nearest,
        novel: candidate.novel,
        taste: candidate.taste,
        score: candidate.score,
        valid: candidate.valid,
    }
}

fn problem_text(problem: &Problem) -> String {
    match problem {
        Problem::MissingMust(keyword) => format!("the prompt leaves out the Must keyword “{keyword}”"),
        Problem::MentionsAvoid(keyword) => format!("it mentions the Avoid keyword “{keyword}”"),
        Problem::EmptyField(field) => format!("its {field} is empty"),
        Problem::PromptTooLong => "the prompt is too long".to_string(),
        Problem::MentionsScreen(word) => format!("the prompt says “{word}”, which can make image models paint a computer"),
    }
}

/// Title, summary and echo note as sentences (no rating: hosts say that in their own language).
fn description(generation: &Generation) -> String {
    let mut parts: Vec<String> = Vec::new();
    let pieces =
        [&generation.concept.title, &generation.concept.summary].into_iter().chain(generation.echo_note.as_ref());
    for piece in pieces {
        let piece = piece.split_whitespace().collect::<Vec<_>>().join(" ");
        if piece.is_empty() {
            continue;
        }
        if piece.ends_with(['.', '!', '?', '…', '。', '！', '？']) {
            parts.push(piece);
        } else {
            parts.push(format!("{piece}."));
        }
    }
    parts.join(" ")
}

/// Feature keys in words (the key itself when no remembered wallpaper names it any more), without repeats.
fn in_words(keys: Vec<String>, labels: &HashMap<String, String>) -> Vec<String> {
    let mut seen = HashSet::new();
    keys.into_iter()
        .map(|key| labels.get(&key).cloned().unwrap_or(key))
        .filter(|label| seen.insert(label.to_lowercase()))
        .collect()
}

fn snapshot(keywords: &[Keyword]) -> Vec<KeywordSnapshot> {
    keywords.iter().map(|keyword| KeywordSnapshot { text: keyword.text.clone(), weight: keyword.weight }).collect()
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// "YYYY" of a Unix time (UTC), for `images/<yyyy>/`.
fn year_of(unix: i64) -> String {
    let month = schedule::month_key(unix);
    month.rsplit_once('-').map_or_else(|| month.clone(), |(year, _)| year.to_string())
}

/// Deletes a file. `Some(bytes freed)` when it is gone (0 if it already was); `None` when it is still there
/// (another app has it open, permissions), so its path must stay recorded.
fn remove_file(path: &str) -> Option<u64> {
    let length = fs::metadata(path).map(|metadata| metadata.len()).unwrap_or(0);
    match fs::remove_file(path) {
        Ok(()) => Some(length),
        Err(error) if error.kind() == ErrorKind::NotFound => Some(0),
        Err(error) => {
            tracing::warn!(%error, "couldn't delete an image file");
            None
        }
    }
}

/// (bytes, files) under `dir`, recursively; unreadable entries count as nothing.
fn dir_usage(dir: &Path) -> (u64, u32) {
    let Ok(entries) = fs::read_dir(dir) else { return (0, 0) };
    let mut bytes = 0u64;
    let mut files = 0u32;
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else { continue };
        if kind.is_dir() {
            let (sub_bytes, sub_files) = dir_usage(&entry.path());
            bytes = bytes.saturating_add(sub_bytes);
            files = files.saturating_add(sub_files);
        } else if kind.is_file() {
            bytes = bytes.saturating_add(entry.metadata().map(|metadata| metadata.len()).unwrap_or(0));
            files = files.saturating_add(1);
        }
    }
    (bytes, files)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn concept(time: &str, weather: &str, season: &str, style: &str, composition: &str) -> Concept {
        Concept {
            time_of_day: time.into(),
            weather: weather.into(),
            season: season.into(),
            style: style.into(),
            composition: composition.into(),
            ..Concept::default()
        }
    }

    #[test]
    fn an_echo_must_visibly_change_a_requested_axis() {
        let original = concept("night", "rain", "autumn", "oil painting", "low horizon");
        let reworded = concept("Night", "rainy", "Autumn", "oil paintings", "low horizon");
        let dawn = concept("dawn", "rain", "autumn", "oil painting", "low horizon");
        assert!(!changed_along_axes(&original, &reworded, &[EchoAxis::TimeOfDay, EchoAxis::Weather]));
        assert!(changed_along_axes(&original, &dawn, &[EchoAxis::Weather, EchoAxis::TimeOfDay]));
        assert!(!changed_along_axes(&original, &dawn, &[EchoAxis::Weather, EchoAxis::Season]));
        let blank = concept("", "rain", "autumn", "oil painting", "low horizon");
        assert!(!changed_along_axes(&original, &blank, &[EchoAxis::TimeOfDay]), "an emptied field isn't a change");
        assert!(changed_along_axes(&original, &reworded, &[EchoAxis::PassageOfTime]), "nothing to check");
        let print = concept("night", "rain", "autumn", "woodblock print", "seen from above");
        assert!(changed_along_axes(&original, &print, &[EchoAxis::Medium, EchoAxis::PassageOfTime]));
        assert!(changed_along_axes(&original, &print, &[EchoAxis::Viewpoint]));
    }

    #[test]
    fn describes_title_summary_and_echo_note_without_the_rating() {
        let mut generation = Generation {
            id: "g".into(),
            created_at: 0,
            trigger: Trigger::Manual,
            status: GenerationStatus::Ok,
            concept: Concept {
                title: "Black ocean, silver structures".into(),
                summary: "A black sea at night with  silver towers.".into(),
                ..Concept::default()
            },
            image_path: None,
            thumb_path: None,
            width: 0,
            height: 0,
            rating: Rating::Liked,
            echo_of: Some("o".into()),
            echo_note: Some("Echo of “Towers” (2 years ago): after a storm".into()),
            text_provider: ProviderKind::Demo,
            text_model: String::new(),
            image_provider: ProviderKind::Demo,
            image_model: String::new(),
            surprise: 0.3,
            keywords: Vec::new(),
            cost_microusd: 0,
            last_shown_at: None,
            shown_count: 0,
            error: None,
            mood_id: None,
            mood_name: None,
        };
        assert_eq!(
            description(&generation),
            "Black ocean, silver structures. A black sea at night with silver towers. Echo of “Towers” (2 years \
             ago): after a storm."
        );
        generation.rating = Rating::Disliked;
        generation.echo_note = None;
        assert_eq!(
            description(&generation),
            "Black ocean, silver structures. A black sea at night with silver towers."
        );
    }

    #[test]
    fn settings_are_clamped_and_checked() {
        let mut settings = Settings { surprise: 3.0, storage_limit_mb: 10, ..Settings::default() };
        settings.text_provider.model = "  gpt-6-luna ".into();
        settings.image_provider.base_url = Some("   ".into());
        settings.comfyui_workflow = Some("  ".into());
        let checked = validated(settings).expect("valid");
        assert_eq!(checked.surprise, 1.0);
        assert_eq!(checked.storage_limit_mb, MIN_STORAGE_LIMIT_MB);
        assert_eq!(checked.text_provider.model, "gpt-6-luna");
        assert_eq!(checked.image_provider.base_url, None);
        assert_eq!(checked.comfyui_workflow, None);
        assert_eq!(validated(Settings { surprise: f32::NAN, ..Settings::default() }).unwrap().surprise, 0.35);

        let selection = |kind, base_url: Option<&str>| ProviderSelection {
            kind,
            model: String::new(),
            base_url: base_url.map(String::from),
        };
        let public_http = Settings {
            text_provider: selection(ProviderKind::Ollama, Some("http://example.com")),
            ..Settings::default()
        };
        assert!(matches!(validated(public_http), Err(AutoPaperError::InvalidInput { .. })));
        let comfy_text = Settings { text_provider: selection(ProviderKind::ComfyUi, None), ..Settings::default() };
        assert!(matches!(validated(comfy_text), Err(AutoPaperError::Unsupported { .. })));
    }

    #[test]
    fn failures_map_to_revisit_reasons() {
        assert_eq!(revisit_reason(&AutoPaperError::Offline), Some(RevisitReason::Offline));
        let unavailable = AutoPaperError::unavailable(
            ProviderKind::OpenAi,
            crate::error::ProviderUnavailableReason::ServerError,
            "HTTP 503",
        );
        assert_eq!(revisit_reason(&unavailable), Some(RevisitReason::ProviderFailed));
        assert_eq!(
            revisit_reason(&AutoPaperError::Refused { provider: ProviderKind::OpenAi }),
            Some(RevisitReason::ProviderFailed)
        );
        assert_eq!(revisit_reason(&AutoPaperError::InvalidKey { provider: ProviderKind::OpenAi }), None);
        assert_eq!(revisit_reason(&AutoPaperError::Cancelled), None);
    }

    #[test]
    fn only_comfyuis_own_failures_need_setup() {
        let node_failed = AutoPaperError::InvalidResponse { detail: "ComfyUI couldn't paint with Qwen-Image 2.1: …".into() };
        let no_workflow = invalid(InvalidInputReason::Other, "AutoPaper has no ComfyUI workflow for x");
        assert!(!node_failed.is_transient());
        assert!(needs_setup(ProviderKind::ComfyUi, &node_failed));
        assert!(needs_setup(ProviderKind::ComfyUi, &no_workflow));
        // Waiting can fix these: ComfyUI not running or stopped, and any hosted service's odd answer.
        for passing in [
            AutoPaperError::unavailable(ProviderKind::ComfyUi, crate::error::ProviderUnavailableReason::NotRunning, "x"),
            AutoPaperError::unavailable(ProviderKind::ComfyUi, crate::error::ProviderUnavailableReason::Stopped, "x"),
            AutoPaperError::Offline,
        ] {
            assert!(!needs_setup(ProviderKind::ComfyUi, &passing), "{passing:?}");
        }
        assert!(!needs_setup(ProviderKind::OpenAi, &node_failed));
        assert!(!needs_setup(ProviderKind::OpenAiCompatible, &node_failed));
    }

    #[test]
    fn a_model_name_from_a_server_is_kept_only_when_plausible() {
        assert_eq!(answered_model(" gpt-6-luna-2026-09-22 ", "gpt-6-luna"), "gpt-6-luna-2026-09-22");
        assert_eq!(answered_model("hf.co/org/model:Q4_K_M", "m"), "hf.co/org/model:Q4_K_M");
        assert_eq!(answered_model("", "m"), "m");
        assert_eq!(answered_model("m\u{1b}]52;c;QUJD\u{7}", "m"), "m");
        assert_eq!(answered_model(&"x".repeat(129), "m"), "m");
    }

    #[test]
    fn images_go_in_year_folders() {
        assert_eq!(year_of(1_791_216_000), "2026");
        assert_eq!(year_of(0), "1970");
    }

    #[test]
    fn console_preserves_prompt_and_models_but_excludes_nested_credentials_and_image_bytes() {
        let body = serde_json::json!({
            "model": "gemini-3.1-flash-image",
            "prompt": "Watercolour of a lighthouse at dawn.",
            "nested": {"api_key": "private-value", "password": "another-secret"},
            "data": [{"b64_json": "pixel-data", "inlineData": {"data": "other-pixels"}}],
            "url": "https://example.test/image?token=private-query#secret-fragment",
            "error": "bad key sk-live-0123456789abcdefghijkl"
        }).to_string();
        let safe = console_body_with_secrets(body.as_bytes(), &[]);
        assert!(safe.contains("Watercolour of a lighthouse") && safe.contains("gemini-3.1-flash-image"));
        for secret in ["private-value", "another-secret", "pixel-data", "other-pixels", "private-query", "secret-fragment", "sk-live-"] {
            assert!(!safe.contains(secret), "leaked {secret}: {safe}");
        }
        assert!(safe.contains("[redacted]") && safe.contains("[image bytes omitted]"));
        assert_eq!(console_body_with_secrets(b"not JSON", &[]), "[non-JSON response: 8 bytes]");
    }

    #[test]
    fn console_caps_untrusted_payloads_and_marks_truncation() {
        let text = console_text(&"word ".repeat(20_000));
        assert!(text.contains("[Console detail truncated"));
        assert!(text.len() < 66_000);
        assert_eq!(console_text("safe\u{1b}\ntext"), "safe\ntext");
    }

    #[test]
    fn console_redacts_opaque_header_credentials_even_when_echoed_as_plain_text() {
        let secrets = vec!["Bearer opaque-provider-key".into(), "opaque-provider-key".into()];
        let detail = console_text_with_secrets("Provider rejected opaque-provider-key (Bearer opaque-provider-key).", &secrets);
        assert!(!detail.contains("opaque-provider-key"));
        assert!(detail.contains("Provider rejected [redacted]"));
        let body = br#"{"output":"opaque-provider-key","prompt":"lighthouse at dawn"}"#;
        let safe = console_body_with_secrets(body, &secrets);
        assert!(safe.contains("lighthouse at dawn") && !safe.contains("opaque-provider-key"));
    }

}
