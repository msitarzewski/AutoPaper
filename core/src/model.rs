//! The types every surface shares: keywords, settings, concepts, generations.
//!
//! These cross the FFI boundary (UniFFI records and enums), so they stay plain: owned strings,
//! integers, vectors. Timestamps are Unix seconds (`i64`); money is micro-US-dollars (`u64`).

use serde::{Deserialize, Serialize};

/// How a keyword constrains the wallpaper.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "snake_case")]
pub enum KeywordWeight {
    /// Always part of the scene.
    Must,
    /// Used some of the time; the agent chooses.
    Maybe,
    /// Never part of the scene, and never mentioned in the prompt.
    Avoid,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, uniffi::Record)]
pub struct Keyword {
    pub id: String,
    /// Trimmed, single-spaced, at most [`Keyword::MAX_LEN`] characters.
    pub text: String,
    pub weight: KeywordWeight,
    pub position: u32,
    pub created_at: i64,
}

impl Keyword {
    pub const MAX_LEN: usize = 40;
    pub const MAX_COUNT: usize = 64;
}

/// A mood: a name, its own keywords (with weights and order) and its own Surprise. Exactly one mood is active;
/// the keyword methods without a mood (`keywords`, `add_keyword`) and `Settings::surprise` act on it. Everything
/// else (cadence, providers, budget, quiet period, echoes) is shared by all moods, and so are memory (novelty) and
/// taste.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, uniffi::Record)]
pub struct Mood {
    pub id: String,
    /// Trimmed, single-spaced, 1–[`Mood::MAX_NAME_LEN`] characters, unique (case-insensitive).
    pub name: String,
    /// Order in the person's list, 0..n without gaps.
    pub position: u32,
    /// 0 = faithful to the keywords, 1 = wild.
    pub surprise: f32,
    pub created_at: i64,
    /// Its keywords, in order.
    pub keywords: Vec<Keyword>,
    /// The mood in use now (exactly one is).
    pub active: bool,
}

impl Mood {
    pub const MAX_NAME_LEN: usize = 40;
}

/// What one mood has made, for a summary of every mood (`Engine::mood_stats`). It counts what History lists for
/// the mood: finished wallpapers not cleared from History (pruned ones too: they're still listed).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, uniffi::Record)]
pub struct MoodStats {
    pub mood_id: String,
    /// Wallpapers made under it.
    pub wallpapers: u32,
    pub liked: u32,
    pub disliked: u32,
    /// How many of them are echoes.
    pub echoes: u32,
    /// When the newest was made (Unix seconds); `None` before its first.
    pub last_made_at: Option<i64>,
    /// Its newest wallpapers, newest first, at most [`MoodStats::LATEST`].
    pub latest: Vec<Generation>,
}

impl MoodStats {
    pub const LATEST: usize = 4;
}

/// How many wallpapers were made under one mood on one day (`Engine::activity`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct DayCount {
    /// The day's start, as the host gave it.
    pub day_start: i64,
    /// The mood they were made under; `None` for wallpapers whose mood was deleted (or made before moods).
    pub mood_id: Option<String>,
    pub count: u32,
}

impl DayCount {
    /// `activity` takes at most this many days.
    pub const MAX_DAYS: usize = 1000;
}

/// A keyword as it was when a wallpaper was made (kept with the generation).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, uniffi::Record)]
pub struct KeywordSnapshot {
    pub text: String,
    pub weight: KeywordWeight,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "snake_case")]
pub enum Rating {
    Disliked,
    Unrated,
    Liked,
}

impl Rating {
    pub fn as_i8(self) -> i8 {
        match self {
            Rating::Disliked => -1,
            Rating::Unrated => 0,
            Rating::Liked => 1,
        }
    }

    pub fn from_i8(value: i8) -> Self {
        match value {
            v if v < 0 => Rating::Disliked,
            0 => Rating::Unrated,
            _ => Rating::Liked,
        }
    }
}

/// Why a wallpaper was made.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "snake_case")]
pub enum Trigger {
    /// The schedule came due.
    Scheduled,
    /// "New Wallpaper Now".
    Manual,
    /// The person disliked the one showing and "Replace wallpapers I dislike" is on.
    DislikeReplace,
    /// "Make an Echo" on a past wallpaper.
    EchoRequest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "snake_case")]
pub enum GenerationStatus {
    Ok,
    Failed,
    /// The provider declined the prompt (content policy).
    Refused,
}

/// The structured scene the text model composes from the keywords. `summary` doubles as the
/// accessible description of the wallpaper.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, uniffi::Record)]
pub struct Concept {
    pub title: String,
    pub summary: String,
    pub setting: String,
    pub subject: String,
    pub elements: Vec<String>,
    pub time_of_day: String,
    pub weather: String,
    pub season: String,
    pub mood: Vec<String>,
    pub palette: Vec<String>,
    pub style: String,
    pub composition: String,
    pub keywords_used: Vec<String>,
    pub wildcards: Vec<String>,
    pub prompt: String,
}

/// One wallpaper the agent made (or tried to make).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, uniffi::Record)]
pub struct Generation {
    pub id: String,
    pub created_at: i64,
    pub trigger: Trigger,
    pub status: GenerationStatus,
    pub concept: Concept,
    /// The original image. `None` once pruned by the storage limit (the memory is kept).
    pub image_path: Option<String>,
    pub thumb_path: Option<String>,
    pub width: u32,
    pub height: u32,
    pub rating: Rating,
    /// The generation this one echoes, if it is an echo.
    pub echo_of: Option<String>,
    /// One line, e.g. "Echo of “Black ocean, silver structures” (March 2024): after a storm, at sunrise."
    pub echo_note: Option<String>,
    pub text_provider: ProviderKind,
    pub text_model: String,
    pub image_provider: ProviderKind,
    pub image_model: String,
    /// Surprise level used, 0–1.
    pub surprise: f32,
    pub keywords: Vec<KeywordSnapshot>,
    /// Estimated cost of the text and image calls together.
    pub cost_microusd: u64,
    pub last_shown_at: Option<i64>,
    pub shown_count: u32,
    /// Why it failed, for failed/refused generations (no secrets, ever).
    pub error: Option<String>,
    /// The mood it was made under; `None` for wallpapers from before moods (the migration gives those the first
    /// mood, so in practice only rows another build wrote).
    pub mood_id: Option<String>,
    /// That mood's name now; `None` once the mood is deleted.
    pub mood_name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    OpenAi,
    Google,
    Ollama,
    OpenAiCompatible,
    ComfyUi,
    /// Draws gradients locally; tests, `autopaper simulate`, and trying the app without a key.
    Demo,
}

impl ProviderKind {
    pub fn writes_concepts(self) -> bool {
        matches!(self, Self::OpenAi | Self::Google | Self::Ollama | Self::OpenAiCompatible | Self::Demo)
    }

    pub fn makes_images(self) -> bool {
        matches!(self, Self::OpenAi | Self::Google | Self::OpenAiCompatible | Self::ComfyUi | Self::Demo)
    }

    /// Runs on the person's own machine or network: no key, no cost.
    pub fn is_local(self) -> bool {
        matches!(self, Self::Ollama | Self::ComfyUi | Self::Demo)
    }

    /// The `SecretStore` account holding a hosted provider's key. OpenAI-compatible keys belong to one server,
    /// so their account depends on its address: see [`secret_account_for`].
    pub fn secret_account(self) -> Option<&'static str> {
        match self {
            Self::OpenAi => Some("openai.api_key"),
            Self::Google => Some("google.api_key"),
            Self::OpenAiCompatible | Self::Ollama | Self::ComfyUi | Self::Demo => None,
        }
    }
}

/// Prefix of an OpenAI-compatible server's key account; the server's origin follows the `@`.
pub const OPENAI_COMPATIBLE_ACCOUNT_PREFIX: &str = "openai_compatible.api_key@";

/// The `SecretStore` account that holds the key for this selection, or `None` when its provider takes no key.
/// Hosts save the key the person types under this account.
///
/// An OpenAI-compatible key is tied to the server it was entered for: the account names the server's origin
/// (`openai_compatible.api_key@https://api.example.com`, port included when it isn't the default), so changing
/// the address never sends an old key to a new server. No valid address → `None` (no key is sent).
#[uniffi::export]
pub fn secret_account_for(selection: ProviderSelection) -> Option<String> {
    match selection.kind {
        ProviderKind::OpenAiCompatible => {
            let base = selection.base_url.as_deref()?.trim();
            let origin = url::Url::parse(base).ok()?.origin();
            origin.is_tuple().then(|| format!("{OPENAI_COMPATIBLE_ACCOUNT_PREFIX}{}", origin.ascii_serialization()))
        }
        kind => kind.secret_account().map(str::to_string),
    }
}

/// Which job a provider is being asked about: writing concepts (ideas) or painting images.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum ProviderJob {
    Concepts,
    Images,
}

/// Surprise in words, by the composer's own bands (`composer::surprise_band`): Faithful below 0.25, Fresh below
/// 0.5, Adventurous below 0.75, Wild from there. Each band allows one more wildcard (0–3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum SurpriseBand {
    Faithful,
    Fresh,
    Adventurous,
    Wild,
}

/// Which provider and model to use for one job (writing concepts, or making images).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, uniffi::Record)]
pub struct ProviderSelection {
    pub kind: ProviderKind,
    /// Empty = the provider's default model.
    pub model: String,
    /// Local and OpenAI-compatible providers only.
    pub base_url: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "snake_case")]
pub enum Cadence {
    Hourly,
    Every3Hours,
    Every6Hours,
    Every12Hours,
    Daily,
    Weekly,
    /// Only when the person asks.
    Manual,
}

impl Cadence {
    pub fn interval_secs(self) -> Option<i64> {
        const HOUR: i64 = 3600;
        match self {
            Self::Hourly => Some(HOUR),
            Self::Every3Hours => Some(3 * HOUR),
            Self::Every6Hours => Some(6 * HOUR),
            Self::Every12Hours => Some(12 * HOUR),
            Self::Daily => Some(24 * HOUR),
            Self::Weekly => Some(7 * 24 * HOUR),
            Self::Manual => None,
        }
    }
}

/// How long before a similar idea may appear again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "snake_case")]
pub enum QuietPeriod {
    OneMonth,
    ThreeMonths,
    SixMonths,
    OneYear,
    TwoYears,
}

impl QuietPeriod {
    pub fn days(self) -> i64 {
        match self {
            Self::OneMonth => 30,
            Self::ThreeMonths => 91,
            Self::SixMonths => 182,
            Self::OneYear => 365,
            Self::TwoYears => 730,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "snake_case")]
pub enum EchoFrequency {
    Off,
    Rarely,
    Sometimes,
    Often,
}

impl EchoFrequency {
    /// Chance that a scheduled wallpaper is an echo, when an original is eligible.
    pub fn chance(self) -> f64 {
        match self {
            Self::Off => 0.0,
            Self::Rarely => 0.05,
            Self::Sometimes => 0.12,
            Self::Often => 0.25,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "snake_case")]
pub enum ImageQuality {
    /// Cheaper and faster.
    Standard,
    /// The provider's best.
    High,
}

/// What to show when a new wallpaper can't be made (over budget, offline, provider down).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "snake_case")]
pub enum Fallback {
    /// Bring back a wallpaper the person liked (costs nothing).
    RevisitLiked,
    KeepCurrent,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, uniffi::Record)]
pub struct Settings {
    /// 0 = faithful to the keywords, 1 = wild. The active mood's (`Mood::surprise`): reading it reads that mood's,
    /// saving it sets that mood's.
    pub surprise: f32,
    pub cadence: Cadence,
    pub paused: bool,
    pub quiet_period: QuietPeriod,
    pub echoes: EchoFrequency,
    pub text_provider: ProviderSelection,
    pub image_provider: ProviderSelection,
    pub image_quality: ImageQuality,
    /// Monthly cap on estimated spend, in US cents. `None` = no cap.
    pub monthly_budget_cents: Option<u32>,
    pub fallback: Fallback,
    pub replace_disliked: bool,
    pub set_lock_screen: bool,
    /// Image files beyond this are pruned, oldest unliked first. Memory is always kept.
    pub storage_limit_mb: u32,
    /// ComfyUI workflow (API format) with `{{prompt}}`, `{{width}}`, `{{height}}`, `{{seed}}`.
    pub comfyui_workflow: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            surprise: 0.35,
            cadence: Cadence::Daily,
            paused: false,
            quiet_period: QuietPeriod::SixMonths,
            echoes: EchoFrequency::Sometimes,
            text_provider: ProviderSelection { kind: ProviderKind::Demo, model: String::new(), base_url: None },
            image_provider: ProviderSelection { kind: ProviderKind::Demo, model: String::new(), base_url: None },
            image_quality: ImageQuality::High,
            monthly_budget_cents: Some(500),
            fallback: Fallback::RevisitLiked,
            replace_disliked: true,
            set_lock_screen: true,
            storage_limit_mb: 2048,
            comfyui_workflow: None,
        }
    }
}

/// Where a generation is, for progress UI ("Composing…", "Painting…").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
pub enum ProgressStage {
    Composing,
    CheckingMemory,
    Generating,
    Downloading,
    Rendering,
    Done,
}

/// Progress of a generation in more detail than its stage, for a determinate ring and "about 6 minutes left"
/// (`ProgressDetailObserver`). `fraction` and `seconds_left` are set only while painting, and only when
/// something real backs them: the painter's own step progress (ComfyUI), or how long this painter took before on
/// this computer (the learned estimate). Otherwise they are `None` and hosts show an indeterminate indicator.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, uniffi::Record)]
pub struct ProgressDetail {
    pub stage: ProgressStage,
    /// 0–1 of this painting, never going backwards; at most 0.99 until the stage moves on, 1 at `Done`.
    pub fraction: Option<f32>,
    /// Seconds until the picture arrives: the sampler's remaining steps at the pace measured in this run once
    /// steps are reported, else the learned estimate minus the time so far. `None` when nothing backs it (or the
    /// estimate has run out); 0 when only decoding and saving are left.
    pub seconds_left: Option<u32>,
}

/// A display to render for, in physical pixels.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct DisplayTarget {
    /// Host's stable identifier (display UUID, monitor device path, connector name).
    pub id: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, uniffi::Record)]
pub struct SpendSummary {
    /// "YYYY-MM"
    pub month: String,
    pub spent_microusd: u64,
    pub images: u32,
    pub budget_cents: Option<u32>,
    /// Estimated cost of one more wallpaper with the current settings.
    pub per_image_microusd: u64,
    /// Estimated cost of a month at the current cadence.
    pub monthly_estimate_microusd: u64,
}

/// What the agent has learned, for "What AutoPaper has learned" in the UI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, uniffi::Record)]
pub struct TasteSummary {
    pub liked: Vec<String>,
    pub disliked: Vec<String>,
    pub ratings: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, uniffi::Record)]
pub struct ModelInfo {
    pub id: String,
    pub display_name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, uniffi::Record)]
pub struct StorageUsage {
    pub image_bytes: u64,
    pub generations: u32,
    pub images_on_disk: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
pub enum HistoryFilter {
    All,
    Liked,
    Disliked,
    Echoes,
}
