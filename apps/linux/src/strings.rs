//! Everything AutoPaper says, in GNOME's sentence case and AudioPaper's voice: plain, specific, short; no hype,
//! no exclamation marks. Engine errors become sentences here (the core's `detail` strings are for logs).
//! GNOME calls an app's settings "Preferences", so these sentences do too (the spec's "Settings").

use autopaper_core::{
    AutoPaperError, Cadence, EchoFrequency, Fallback, ImageQuality, InvalidInputReason, Keyword, KeywordWeight, Mood,
    ProgressStage, ProviderJob, ProviderKind, ProviderUnavailableReason, QuietPeriod, Rating, RevisitReason,
    SurpriseBand, default_model,
};
use gtk::glib;
use gtk::prelude::*;

pub fn provider_name(kind: ProviderKind) -> &'static str {
    match kind {
        ProviderKind::OpenAi => "OpenAI",
        ProviderKind::Google => "Google Gemini",
        ProviderKind::Ollama => "Ollama",
        ProviderKind::OpenAiCompatible => "Your OpenAI-compatible server",
        ProviderKind::ComfyUi => "ComfyUI",
        ProviderKind::Demo => "Demo",
    }
}

/// The provider's name inside a sentence ("your OpenAI-compatible server" in lower case).
pub fn provider_in_sentence(kind: ProviderKind) -> String {
    match kind {
        ProviderKind::OpenAiCompatible => "your OpenAI-compatible server".into(),
        other => provider_name(other).into(),
    }
}

/// An engine error as one plain sentence that says where to fix it (a toast outside Preferences). `fallback`
/// decides how the budget sentence ends.
pub fn error_sentence(error: &AutoPaperError, fallback: Fallback) -> String {
    sentence(error, fallback, Context::Main)
}

/// The same problem shown as a link to the field that fixes it (the Now view's notice, a notification): it says
/// what to do, not where ("Add your OpenAI key", not "… in Preferences → Keys"); the link goes there.
pub fn linked_sentence(error: &AutoPaperError, fallback: Fallback) -> String {
    sentence(error, fallback, Context::Linked)
}

/// A missing or refused key, as the link that goes to its field: "Add your OpenAI key", "Check your OpenAI key".
pub fn key_link(kind: ProviderKind, refused: bool) -> String {
    let name = provider_key_name(kind);
    if refused { format!("Check your {name} key") } else { format!("Add your {name} key") }
}

fn sentence(error: &AutoPaperError, _fallback: Fallback, context: Context) -> String {
    match error {
        AutoPaperError::MissingKey { provider } if context == Context::Linked => key_link(*provider, false),
        AutoPaperError::MissingKey { provider } => {
            format!("Add your {} key in Preferences to start.", provider_key_name(*provider))
        }
        AutoPaperError::InvalidKey { provider } if context == Context::Linked => key_link(*provider, true),
        AutoPaperError::InvalidKey { provider } => format!(
            "{} didn't accept your key. Check it in Preferences → Keys.",
            capitalized(&provider_in_sentence(*provider))
        ),
        AutoPaperError::ProviderUnavailable { provider, reason, .. } => unavailable(*provider, *reason, context),
        AutoPaperError::RateLimited { provider, retry_after_secs } => {
            let minutes = (retry_after_secs.div_ceil(60)).max(1);
            let unit = if minutes == 1 { "minute" } else { "minutes" };
            format!(
                "{} asked AutoPaper to wait; it'll try again in {minutes} {unit}.",
                capitalized(&provider_in_sentence(*provider))
            )
        }
        AutoPaperError::Refused { provider } => format!(
            "{} declined to paint this idea; AutoPaper tried a gentler one.",
            capitalized(&provider_in_sentence(*provider))
        ),
        AutoPaperError::Unsupported { provider, .. } => format!(
            "{} can't do this job. Choose another provider{}.",
            capitalized(&provider_in_sentence(*provider)),
            if context == Context::Linked { "" } else { " in Preferences" }
        ),
        AutoPaperError::BudgetReached { .. } => "The next wallpaper would exceed this month's budget. Your current wallpaper is kept. Raise the budget in Preferences, or wait for the monthly reset.".into(),
        AutoPaperError::Offline => "You're offline. AutoPaper will try again later.".into(),
        AutoPaperError::InvalidResponse { .. } => {
            "The provider's answer couldn't be used. AutoPaper will try again later.".into()
        }
        // One line naming the keyword, in every context: the Now view makes it the link to the mood (rule 6a), and
        // waiting won't fix it, so it never says "try again".
        AutoPaperError::KeywordNotFollowed { keyword, weight, .. } => keyword_not_followed(keyword, *weight),
        // One line; outside a link it says where it's fixed. Waiting won't fix it, so it never says "try again".
        AutoPaperError::PaintingFailed { provider, model, .. } => match context {
            Context::Main => format!("{} Check it in Preferences → Providers.", painting_failed(*provider, model)),
            Context::Preferences | Context::Linked => painting_failed(*provider, model),
        },
        // Worded by the typed reason only: `detail` is English, for logs, and never shown.
        AutoPaperError::InvalidInput { reason, .. } => invalid_input(*reason, context),
        AutoPaperError::NotFound => "That wallpaper isn't there any more.".into(),
        AutoPaperError::NothingToRevisit => "There's no liked wallpaper to bring back yet.".into(),
        AutoPaperError::Storage { .. } => "AutoPaper couldn't read or write its files.".into(),
        AutoPaperError::Cancelled => "Stopped.".into(),
        AutoPaperError::Internal { .. } => "Something went wrong inside AutoPaper.".into(),
    }
}

/// A provider problem in Preferences (Test, the model list): the same facts, without sending the person to
/// Preferences, where they already are.
pub fn provider_problem(error: &AutoPaperError) -> String {
    match error {
        AutoPaperError::MissingKey { provider } => {
            format!("{} needs a key: add it on the Keys page.", capitalized(&provider_in_sentence(*provider)))
        }
        AutoPaperError::InvalidKey { provider } => format!(
            "{} didn't accept the key. Check it on the Keys page.",
            capitalized(&provider_in_sentence(*provider))
        ),
        AutoPaperError::ProviderUnavailable { provider, reason, .. } => {
            unavailable(*provider, *reason, Context::Preferences)
        }
        AutoPaperError::RateLimited { provider, retry_after_secs } => {
            let minutes = (retry_after_secs.div_ceil(60)).max(1);
            let unit = if minutes == 1 { "minute" } else { "minutes" };
            format!("{} asked AutoPaper to wait {minutes} {unit}.", capitalized(&provider_in_sentence(*provider)))
        }
        AutoPaperError::Offline => "You're offline.".into(),
        AutoPaperError::Unsupported { provider, .. } => {
            format!("{} can't do this job.", capitalized(&provider_in_sentence(*provider)))
        }
        AutoPaperError::InvalidInput { reason, .. } => invalid_input(*reason, Context::Preferences),
        AutoPaperError::PaintingFailed { provider, model, .. } => painting_failed(*provider, model),
        other => error_sentence(other, autopaper_core::Fallback::KeepCurrent),
    }
}

/// `KeywordNotFollowed` in one line that names the keyword: "The writing model kept leaving out “lighthouse”." for a
/// Must, "The writing model kept including “people”, which this mood avoids." for an Avoid. (A Maybe is never the
/// reason, but it's worded like a Must if the core ever says so.)
pub fn keyword_not_followed(keyword: &str, weight: KeywordWeight) -> String {
    match weight {
        KeywordWeight::Avoid => format!("The writing model kept including “{keyword}”, which this mood avoids."),
        KeywordWeight::Must | KeywordWeight::Maybe => format!("The writing model kept leaving out “{keyword}”."),
    }
}

/// `PaintingFailed` in one line: "ComfyUI couldn't paint with Qwen-Image 2.1." (the core names the model in words;
/// it's empty when the person's own workflow loads a model AutoPaper can't name).
pub fn painting_failed(provider: ProviderKind, model: &str) -> String {
    match model.trim() {
        "" => format!("{} couldn't paint this wallpaper.", capitalized(&provider_in_sentence(provider))),
        model => format!("{} couldn't paint with {model}.", capitalized(&provider_in_sentence(provider))),
    }
}

/// The link under a painting failure, to the painting settings (docs/app-spec.md 6a: one line plus a link).
pub fn painting_link(provider: ProviderKind) -> String {
    match provider {
        ProviderKind::OpenAiCompatible => "Check your server's painting settings".into(),
        other => format!("Check {}'s settings", provider_name(other)),
    }
}

/// Where a sentence is shown: a toast outside Preferences (`Main`), which says where to fix things; inside
/// Preferences, next to the field (`Preferences`), which doesn't; or as a link to the field that fixes it (`Linked`:
/// the Now view's notice, a notification), which says what to do and lets the link go there (docs/app-spec.md 6a).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Context {
    Main,
    Preferences,
    Linked,
}

/// `ProviderUnavailable`, by its typed reason. The schedule tries again after each of them.
fn unavailable(provider: ProviderKind, reason: ProviderUnavailableReason, context: Context) -> String {
    let subject = capitalized(&provider_in_sentence(provider));
    let later = match context {
        Context::Main | Context::Linked => " AutoPaper will try again later.",
        Context::Preferences => " Try again later.",
    };
    let not_answering = || match context {
        Context::Main => format!("{subject} isn't answering. Check that it's running and its address in Preferences."),
        Context::Linked => format!("{subject} isn't answering. Check that it's running, and its address."),
        Context::Preferences => format!("{subject} isn't answering. Is it running, at that address?"),
    };
    match reason {
        ProviderUnavailableReason::NotRunning => not_answering(),
        ProviderUnavailableReason::TimedOut => format!("{subject} took too long to answer.{later}"),
        ProviderUnavailableReason::ServerError => format!("{subject} had a problem on its side.{later}"),
        ProviderUnavailableReason::Stopped => format!("The job was stopped in {}.", provider_name(provider)),
        ProviderUnavailableReason::Other if provider.is_local() => not_answering(),
        ProviderUnavailableReason::Other => format!("{subject} isn't available right now.{later}"),
    }
}

/// `InvalidInput`, by its typed reason; each sentence names the field to change. `DisplaySizeInvalid` and
/// `Other` are AutoPaper's own mistakes, said generically.
pub fn invalid_input(reason: InvalidInputReason, context: Context) -> String {
    let providers = match context {
        Context::Main => " in Preferences → Providers",
        Context::Preferences | Context::Linked => "",
    };
    match reason {
        InvalidInputReason::KeywordEmpty => "A keyword needs at least one letter or number.".into(),
        InvalidInputReason::KeywordTooLong => format!("Keywords can be up to {} characters.", Keyword::MAX_LEN),
        InvalidInputReason::TooManyKeywords => {
            format!("You can have up to {} keywords. Remove one to add another.", Keyword::MAX_COUNT)
        }
        InvalidInputReason::DuplicateKeyword => "Another keyword already has this text.".into(),
        InvalidInputReason::AddressMissing => format!("Enter the server's address{providers}."),
        InvalidInputReason::AddressNotAllowed => format!(
            "AutoPaper can't use {}. Use https, or http only for this computer or your local network, with no \
             user name or password in it.",
            match context {
                Context::Main => "the server address in Preferences → Providers",
                Context::Linked => "your server's address",
                Context::Preferences => "that address",
            }
        ),
        InvalidInputReason::AddressInvalid => format!(
            "{} isn't a complete web address. Include http:// or https:// and the server's name.",
            match context {
                Context::Main => "The server address in Preferences → Providers",
                Context::Linked => "Your server's address",
                Context::Preferences => "That",
            }
        ),
        InvalidInputReason::NoModels => match context {
            Context::Main => "The server has no models to use yet. Download one, or choose a model in Preferences → \
                              Providers."
                .into(),
            Context::Linked => "The server has no models to use yet. Download one, or choose a model.".into(),
            Context::Preferences => "The server has no models to use yet. Download one, then refresh the list.".into(),
        },
        InvalidInputReason::WorkflowNeedsPrompt => format!(
            "Your ComfyUI workflow needs a {{{{prompt}}}} placeholder where AutoPaper puts the scene. Add it in \
             ComfyUI, then choose the workflow again{providers}."
        ),
        InvalidInputReason::WorkflowNotApiFormat => format!(
            "Your ComfyUI workflow is in ComfyUI's editor format. In ComfyUI, choose Export (API), then use that \
             file{providers}."
        ),
        InvalidInputReason::WorkflowInvalid => format!(
            "Your ComfyUI workflow isn't valid once AutoPaper fills in {{{{prompt}}}}, {{{{width}}}}, {{{{height}}}} \
             and {{{{seed}}}}. Check it in ComfyUI and choose it again{providers}."
        ),
        InvalidInputReason::NothingToEcho => "Only a finished wallpaper can have an echo.".into(),
        InvalidInputReason::MoodNameEmpty => "A mood needs a name.".into(),
        InvalidInputReason::MoodNameTooLong => format!("Mood names can be up to {} characters.", Mood::MAX_NAME_LEN),
        InvalidInputReason::DuplicateMoodName => "Another mood already has this name.".into(),
        InvalidInputReason::LastMood => "AutoPaper always keeps one mood, so the last one can't be deleted.".into(),
        InvalidInputReason::DisplaySizeInvalid | InvalidInputReason::Other => {
            "Something went wrong inside AutoPaper.".into()
        }
    }
}

/// True for the reasons that are about a provider's server address (the field to mark in Preferences).
pub fn is_address_problem(error: &AutoPaperError) -> bool {
    matches!(
        error,
        AutoPaperError::InvalidInput {
            reason: InvalidInputReason::AddressMissing
                | InvalidInputReason::AddressInvalid
                | InvalidInputReason::AddressNotAllowed,
            ..
        }
    )
}

/// The model picker's blank choice: "Default (gpt-6-luna)", from the core's `default_model`. Ollama and
/// OpenAI-compatible servers have no fixed default (they use the first model they list), and a ComfyUI workflow
/// of the person's own always loads its own model. ComfyUI's default is named in words ("Default (Z-Image Turbo)"),
/// also while the server can't be asked for its list.
pub fn default_model_label(kind: ProviderKind, job: ProviderJob, own_workflow: bool) -> String {
    if kind == ProviderKind::ComfyUi && own_workflow {
        return "Default (your workflow's model)".into();
    }
    if kind == ProviderKind::ComfyUi
        && job == ProviderJob::Images
        && let Some(name) = autopaper_core::providers::comfyui::bundled_model_name()
    {
        return format!("Default ({name})");
    }
    let model = default_model(kind, job);
    if !model.is_empty() {
        return format!("Default ({model})");
    }
    match kind {
        ProviderKind::Ollama | ProviderKind::OpenAiCompatible => "Default (the server's first model)".into(),
        _ => "Default".into(),
    }
}

fn provider_key_name(kind: ProviderKind) -> &'static str {
    match kind {
        ProviderKind::OpenAi => "OpenAI",
        ProviderKind::Google => "Google Gemini",
        _ => "server's",
    }
}

/// Why a saved wallpaper came back instead of a new one (`None` when the person asked).
pub fn revisit_sentence(reason: RevisitReason) -> Option<&'static str> {
    match reason {
        RevisitReason::Requested => None,
        RevisitReason::OverBudget => Some("This month's budget is spent, so AutoPaper brought back one you liked."),
        RevisitReason::Offline => Some("You're offline, so AutoPaper brought back one you liked."),
        RevisitReason::ProviderFailed => Some("A new one couldn't be made, so AutoPaper brought back one you liked."),
        RevisitReason::ServicesUnavailable => Some(
            "A service is unavailable, so AutoPaper is showing the latest saved wallpaper from this mood. See Console for details.",
        ),
    }
}

/// The progress line before the engine reports its first stage (a scheduled run), and the tray's.
pub const MAKING: &str = "Making a wallpaper…";

/// "New wallpaper: <title>." — said when one is made (the notification's title says the same without the stop).
pub fn new_wallpaper(title: &str) -> String {
    format!("New wallpaper: {title}.")
}

/// Said when a past wallpaper is put back on the desktop from History.
pub fn on_desktop(title: &str) -> String {
    format!("Now on your desktop: {title}.")
}

/// How long is left of a painting, for the progress pill: "a few seconds left", "about 40 seconds left", "about 6
/// minutes left", "about 2 hours left". Never more exact than the estimate behind it.
pub fn time_left(seconds: u32) -> String {
    match seconds {
        0..=10 => "a few seconds left".into(),
        11..=59 => format!("about {} seconds left", seconds.div_ceil(5) * 5),
        60..=89 => "about 1 minute left".into(),
        90..=5399 => format!("about {} minutes left", (seconds + 30) / 60),
        5400.. => format!("about {} hours left", (seconds + 1800) / 3600),
    }
}

/// A duration in words, rounded the way `time_left` rounds: "5 seconds", "1 minute", "9 minutes", "2 hours".
pub fn duration(seconds: u32) -> String {
    match seconds {
        0..=1 => "1 second".into(),
        2..=59 => format!("{seconds} seconds"),
        60..=89 => "1 minute".into(),
        90..=5399 => format!("{} minutes", (seconds + 30) / 60),
        5400.. => format!("{} hours", (seconds + 1800) / 3600),
    }
}

/// What a writer or painter usually takes here, from `estimate` (learned on this computer, never made up):
/// "About 9 minutes per wallpaper on this computer", "About 4 seconds per idea".
pub fn estimate_line(job: ProviderJob, kind: ProviderKind, seconds: u32) -> String {
    let what = match job {
        ProviderJob::Concepts => "per idea",
        ProviderJob::Images => "per wallpaper",
    };
    let place = if kind.is_local() { " on this computer" } else { "" };
    format!("About {} {what}{place}", duration(seconds))
}

/// The progress bar's spoken value: "45 percent, about 2 minutes left".
pub fn progress_spoken(fraction: f32, seconds_left: Option<u32>) -> String {
    let percent = (fraction.clamp(0.0, 1.0) * 100.0).floor() as u32;
    match seconds_left {
        Some(left) => format!("{percent} percent, {}", time_left(left)),
        None => format!("{percent} percent"),
    }
}

/// A mood's keywords in short, for its row in the Moods list: "rain · beach · no people" (an Avoid names what's left
/// out). Never empty.
pub fn mood_keywords(keywords: &[Keyword]) -> String {
    if keywords.is_empty() {
        return "No keywords yet".into();
    }
    keywords
        .iter()
        .map(|keyword| match keyword.weight {
            KeywordWeight::Avoid => format!("no {}", keyword.text),
            _ => keyword.text.clone(),
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

/// A mood as screen readers hear it in lists and menus: "Rainy beach, current mood".
pub fn mood_spoken(mood: &Mood) -> String {
    if mood.active { format!("{}, current mood", mood.name) } else { mood.name.clone() }
}

/// A name no other mood has (the core compares names case-insensitively): `base`, else "`base` 2", "`base` 3", …,
/// shortened so it stays within the core's 40 characters.
pub fn unused_mood_name(base: &str, moods: &[Mood]) -> String {
    let taken = |name: &str| moods.iter().any(|mood| mood.name.to_lowercase() == name.to_lowercase());
    let fit = |text: &str, room: usize| -> String {
        let text: String = text.chars().take(room).collect();
        text.trim_end().to_string()
    };
    let first = fit(base, Mood::MAX_NAME_LEN);
    if !taken(&first) {
        return first;
    }
    (2..).map(|n| {
        let suffix = format!(" {n}");
        format!("{}{suffix}", fit(base, Mood::MAX_NAME_LEN - suffix.chars().count()))
    })
    .find(|name| !taken(name))
    .unwrap_or(first)
}

/// The name of a duplicated mood: "Rainy beach copy" (or "Rainy beach copy 2" when that's taken).
pub fn mood_copy_name(name: &str, moods: &[Mood]) -> String {
    unused_mood_name(&format!("{name} copy"), moods)
}

/// "Rainy beach is the current mood." — said when a mood is put to use.
pub fn mood_in_use(name: &str) -> String {
    format!("{name} is the current mood.")
}

/// The progress line while a wallpaper is being made (`None` once it's done).
pub fn stage_label(stage: ProgressStage) -> Option<&'static str> {
    match stage {
        ProgressStage::CheckingServices => Some("Checking services…"),
        ProgressStage::Composing => Some("Composing an idea…"),
        ProgressStage::CheckingMemory => Some("Checking memory…"),
        ProgressStage::Generating => Some("Painting…"),
        ProgressStage::Downloading => Some("Downloading…"),
        ProgressStage::Rendering => Some("Preparing for your displays…"),
        ProgressStage::Done => None,
    }
}

pub fn rating_words(rating: Rating) -> Option<&'static str> {
    match rating {
        Rating::Liked => Some("Liked"),
        Rating::Disliked => Some("Disliked"),
        Rating::Unrated => None,
    }
}

/// `describe(id)` plus the rating in the app's words ("… Liked."), for screen readers.
pub fn spoken_description(description: &str, rating: Rating) -> String {
    match rating_words(rating) {
        Some(words) if description.is_empty() => format!("{words}."),
        Some(words) => format!("{description} {words}."),
        None => description.to_string(),
    }
}

// ── Which models made a wallpaper ───────────────────────────────────────────────────────────────

/// Names for model ids when the provider's own list hasn't been read (as macOS's `Provenance.knownNames`): Google's
/// from its model list, ComfyUI's from AutoPaper's bundled workflows. OpenAI's list names models by id.
const KNOWN_MODELS: [(&str, &str); 10] = [
    ("gemini-3.1-flash-image", "Nano Banana 2"),
    ("gemini-3.1-flash-lite-image", "Nano Banana 2 Lite"),
    ("gemini-3-pro-image", "Nano Banana Pro"),
    ("gemini-2.5-flash-image", "Nano Banana"),
    ("gemini-3.5-flash-lite", "Gemini 3.5 Flash-Lite"),
    ("gemini-3.1-flash-lite", "Gemini 3.1 Flash-Lite"),
    ("gemini-3.8-flash", "Gemini 3.8 Flash"),
    ("z_image_turbo_bf16.safetensors", "Z-Image Turbo"),
    ("krea2_turbo_fp8_scaled.safetensors", "Krea 2 Turbo"),
    ("qwen_image_2.1_int8_convrot.safetensors", "Qwen-Image 2.1"),
];

/// A model in words: AutoPaper's name for it, else (ComfyUI) its file's name without folders or extension, else its
/// id. Demo, and an empty id (a record from before models were kept), are the provider's name.
pub fn model_in_words(kind: ProviderKind, model: &str) -> String {
    let model = model.trim();
    if kind == ProviderKind::Demo || model.is_empty() {
        return provider_name(kind).to_string();
    }
    if let Some((_, name)) = KNOWN_MODELS.iter().find(|(id, _)| *id == model) {
        return (*name).to_string();
    }
    if kind == ProviderKind::ComfyUi {
        let file = model.rsplit(['/', '\\']).next().unwrap_or(model);
        let stem = file.rsplit_once('.').map(|(stem, _)| stem).filter(|stem| !stem.is_empty()).unwrap_or(file);
        return stem.to_string();
    }
    model.to_string()
}

/// The model and its provider: "Nano Banana 2 (Google Gemini)", "Z-Image Turbo (ComfyUI)", or "Demo" alone.
fn who(kind: ProviderKind, model: &str) -> String {
    let name = model_in_words(kind, model);
    if name == provider_name(kind) { name } else { format!("{name} ({})", provider_name(kind)) }
}

/// Which models made a wallpaper (user, 2026-10-06): "Written by … · Painted by … · 3840×2160 · about $0.04", from the
/// generation's own record (the models the engine used, defaults resolved), so it shows whether a model chosen in
/// Preferences was applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provenance {
    /// "Written by Gemini 3.5 Flash-Lite (Google Gemini)".
    pub written: String,
    /// "Painted by Nano Banana 2 (Google Gemini)": a link to Preferences → Providers where it's shown.
    pub painted: String,
    /// "3840×2160"; `None` when the size wasn't kept.
    pub size: Option<String>,
    /// "about $0.04"; `None` when it cost nothing (local providers, Demo).
    pub cost: Option<String>,
}

impl Provenance {
    pub fn of(generation: &autopaper_core::Generation) -> Self {
        let size = (generation.width > 0 && generation.height > 0)
            .then(|| format!("{}×{}", generation.width, generation.height));
        let hosted = |kind: ProviderKind| matches!(kind, ProviderKind::OpenAi | ProviderKind::Google | ProviderKind::OpenAiCompatible);
        let paid = generation.cost_microusd > 0 && (hosted(generation.text_provider) || hosted(generation.image_provider));
        Self {
            written: format!("Written by {}", who(generation.text_provider, &generation.text_model)),
            painted: format!("Painted by {}", who(generation.image_provider, &generation.image_model)),
            size,
            cost: paid.then(|| format!("about {}", money(generation.cost_microusd))),
        }
    }

    /// The line as shown.
    pub fn text(&self) -> String {
        [Some(self.written.clone()), Some(self.painted.clone()), self.size.clone(), self.cost.clone()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ")
    }

    /// The line as screen readers say it: commas, and "3840 by 2160".
    pub fn spoken(&self) -> String {
        [Some(self.written.clone()), Some(self.painted.clone()), self.size.as_ref().map(|size| size.replace('×', " by ")), self.cost.clone()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// What follows the painter, with its separators (" · 3840×2160 · about $0.04"), for showing the painter as a link
    /// between the two halves.
    pub fn tail(&self) -> String {
        [self.size.as_ref(), self.cost.as_ref()].into_iter().flatten().map(|part| format!(" · {part}")).collect()
    }
}

// ── Moods, summed up ────────────────────────────────────────────────────────────────────────────

/// "Surprise: Fresh (35%)".
pub fn surprise_line(surprise: f32) -> String {
    let surprise = if surprise.is_nan() { 0.0 } else { surprise.clamp(0.0, 1.0) };
    let percent = (surprise * 100.0).round() as u32;
    format!("Surprise: {} ({percent}%)", surprise_band(surprise).0)
}

/// "1 wallpaper", "3 wallpapers".
pub fn count(value: u32, one: &str, many: &str) -> String {
    format!("{value} {}", if value == 1 { one } else { many })
}

/// How long ago, in words, as in "last made 3 hours ago": "just now", "5 minutes ago", "yesterday", "3 days ago",
/// "2 weeks ago", "4 months ago", "2 years ago". Never in the future: a clock set back reads "just now".
pub fn relative_time(unix: i64, now: i64) -> String {
    let seconds = (now - unix).max(0);
    let calendar_days = match (glib::DateTime::from_unix_local(now), glib::DateTime::from_unix_local(unix.min(now))) {
        (Ok(now), Ok(then)) => day_number(&now) - day_number(&then),
        _ => seconds / 86_400,
    };
    match seconds {
        0..60 => "just now".into(),
        60..3600 => format!("{} ago", count((seconds / 60) as u32, "minute", "minutes")),
        _ if calendar_days == 0 || seconds < 6 * 3600 => format!("{} ago", count((seconds / 3600) as u32, "hour", "hours")),
        _ if calendar_days == 1 => "yesterday".into(),
        _ if calendar_days < 7 => format!("{calendar_days} days ago"),
        _ if calendar_days < 35 => format!("{} ago", count((calendar_days / 7) as u32, "week", "weeks")),
        _ if calendar_days < 365 => format!("{} ago", count((calendar_days / 30).max(1) as u32, "month", "months")),
        _ => format!("{} ago", count((calendar_days / 365) as u32, "year", "years")),
    }
}

/// What a mood has made, on one line: "12 wallpapers · 3 liked · last made yesterday", or "Nothing made yet".
/// `separator` is " · " to show and ", " to speak.
pub fn made_line(wallpapers: u32, liked: u32, last_made_at: Option<i64>, separator: &str, now: i64) -> String {
    if wallpapers == 0 {
        return "Nothing made yet".into();
    }
    let mut parts = vec![count(wallpapers, "wallpaper", "wallpapers"), format!("{liked} liked")];
    if let Some(made) = last_made_at {
        parts.push(format!("last made {}", relative_time(made, now)));
    }
    parts.join(separator)
}

/// A mood header's "Last made" tile: "3 hours ago", "Yesterday", or "Not yet".
pub fn last_made_tile(last_made_at: Option<i64>, now: i64) -> String {
    match last_made_at {
        Some(made) => capitalized(&relative_time(made, now)),
        None => "Not yet".into(),
    }
}

pub fn weight_title(weight: KeywordWeight) -> &'static str {
    match weight {
        KeywordWeight::Must => "Must",
        KeywordWeight::Maybe => "Maybe",
        KeywordWeight::Avoid => "Avoid",
    }
}

const WEIGHT_ORDER: [KeywordWeight; 3] = [KeywordWeight::Must, KeywordWeight::Maybe, KeywordWeight::Avoid];

/// A mood's keywords grouped by weight, Must then Maybe then Avoid (each in the mood's order), leaving out weights it
/// has none of: "Must: rain, beach". The summary of every mood shows them this way.
pub fn keyword_groups<'a>(keywords: impl IntoIterator<Item = (&'a str, KeywordWeight)> + Clone) -> Vec<String> {
    WEIGHT_ORDER
        .iter()
        .filter_map(|weight| {
            let words: Vec<&str> =
                keywords.clone().into_iter().filter(|(_, w)| w == weight).map(|(text, _)| text).collect();
            (!words.is_empty()).then(|| format!("{}: {}", weight_title(*weight), words.join(", ")))
        })
        .collect()
}

/// Every mood's keywords by weight, for the summary: "12 Must · 7 Maybe · 3 Avoid keywords".
pub fn keyword_breakdown(moods: &[Mood]) -> String {
    let keywords: Vec<KeywordWeight> = moods.iter().flat_map(|mood| mood.keywords.iter().map(|keyword| keyword.weight)).collect();
    if keywords.is_empty() {
        return "No keywords yet".into();
    }
    let parts: Vec<String> = WEIGHT_ORDER
        .iter()
        .map(|weight| format!("{} {}", keywords.iter().filter(|w| *w == weight).count(), weight_title(*weight)))
        .collect();
    format!("{} {}", parts.join(" · "), if keywords.len() == 1 { "keyword" } else { "keywords" })
}

/// The thumbnails of a mood's latest wallpapers as one spoken line: "Latest wallpapers: Harbour at dusk, Fog".
pub fn spoken_latest(titles: &[String]) -> String {
    match titles {
        [one] => format!("Latest wallpaper: {one}"),
        many => format!("Latest wallpapers: {}", many.join(", ")),
    }
}

pub const CADENCES: [(Cadence, &str); 7] = [
    (Cadence::Hourly, "Every hour"),
    (Cadence::Every3Hours, "Every 3 hours"),
    (Cadence::Every6Hours, "Every 6 hours"),
    (Cadence::Every12Hours, "Every 12 hours"),
    (Cadence::Daily, "Every day"),
    (Cadence::Weekly, "Every week"),
    (Cadence::Manual, "Only when I ask"),
];

/// "at this pace" in the budget estimate.
pub fn cadence_pace(cadence: Cadence) -> &'static str {
    match cadence {
        Cadence::Hourly => "at one an hour",
        Cadence::Every3Hours => "at one every 3 hours",
        Cadence::Every6Hours => "at one every 6 hours",
        Cadence::Every12Hours => "at one every 12 hours",
        Cadence::Daily => "at one a day",
        Cadence::Weekly => "at one a week",
        Cadence::Manual => "when you only ask now and then",
    }
}

pub const QUIET_PERIODS: [(QuietPeriod, &str); 5] = [
    (QuietPeriod::OneMonth, "1 month"),
    (QuietPeriod::ThreeMonths, "3 months"),
    (QuietPeriod::SixMonths, "6 months"),
    (QuietPeriod::OneYear, "1 year"),
    (QuietPeriod::TwoYears, "2 years"),
];

pub const ECHOES: [(EchoFrequency, &str); 4] = [
    (EchoFrequency::Off, "Off"),
    (EchoFrequency::Rarely, "Rarely"),
    (EchoFrequency::Sometimes, "Sometimes"),
    (EchoFrequency::Often, "Often"),
];

pub const FALLBACKS: [(Fallback, &str); 2] =
    [(Fallback::RevisitLiked, "Bring back one I liked"), (Fallback::KeepCurrent, "Keep the current one")];

pub const QUALITIES: [(ImageQuality, &str); 2] = [(ImageQuality::Standard, "Standard"), (ImageQuality::High, "High")];

/// Storage limits offered, in MB.
pub const STORAGE_LIMITS: [(u32, &str); 5] =
    [(256, "256 MB"), (512, "512 MB"), (1024, "1 GB"), (2048, "2 GB"), (5120, "5 GB")];

/// Monthly budgets offered, in cents (`None` = no limit). "Custom" follows these in the list.
pub const BUDGETS: [(Option<u32>, &str); 6] = [
    (None, "No limit"),
    (Some(100), "$1"),
    (Some(200), "$2"),
    (Some(500), "$5"),
    (Some(1000), "$10"),
    (Some(2000), "$20"),
];

/// The Surprise band a value falls in, in words: the band is the core's (`surprise_band`, the composer's own
/// thresholds); the app only names and explains it.
pub fn surprise_band(surprise: f32) -> (&'static str, &'static str) {
    match autopaper_core::surprise_band(surprise) {
        SurpriseBand::Faithful => ("Faithful", "Each keyword as anyone would picture it."),
        SurpriseBand::Fresh => ("Fresh", "Believable scenes, each with one unexpected choice."),
        SurpriseBand::Adventurous => ("Adventurous", "Keywords read freely: unexpected settings, eras and styles."),
        SurpriseBand::Wild => ("Wild", "Keywords as loose inspiration for dreamlike scenes."),
    }
}

/// "35 percent, fresh": the Surprise slider's spoken value.
pub fn surprise_spoken(surprise: f32) -> String {
    let percent = (surprise.clamp(0.0, 1.0) * 100.0).round() as u32;
    format!("{percent} percent, {}", surprise_band(surprise).0.to_lowercase())
}

/// Micro-dollars as "$1.20" (and "under $0.01" for a sliver).
pub fn money(microusd: u64) -> String {
    if microusd > 0 && microusd < 5_000 {
        return "under $0.01".into();
    }
    let cents = (microusd + 5_000) / 10_000;
    format!("${}.{:02}", cents / 100, cents % 100)
}

/// Console costs keep every stored micro-dollar, so small paid requests never look free.
pub fn console_money(microusd: u64) -> String {
    let mut fraction = format!("{:06}", microusd % 1_000_000);
    while fraction.len() > 2 && fraction.ends_with('0') {
        fraction.pop();
    }
    format!("${}.{}", microusd / 1_000_000, fraction)
}

pub fn cents(cents: u32) -> String {
    if cents.is_multiple_of(100) { format!("${}", cents / 100) } else { format!("${}.{:02}", cents / 100, cents % 100) }
}

/// "Next new wallpaper at 21:00" / "… tomorrow at 9:00 AM" / "… on Friday at …" / "… soon".
pub fn next_due_sentence(next_due: Option<i64>, paused: bool, cadence: Cadence) -> String {
    if paused {
        return "Paused".into();
    }
    if cadence == Cadence::Manual {
        return "Only when you ask".into();
    }
    let Some(next) = next_due else {
        return "Only when you ask".into();
    };
    let (Ok(now), Ok(when)) = (glib::DateTime::now_local(), glib::DateTime::from_unix_local(next)) else {
        return "Next new wallpaper soon".into();
    };
    if when.to_unix() <= now.to_unix() + 30 {
        return "Next new wallpaper soon".into();
    }
    let time = clock_time(&when);
    let days = day_number(&when) - day_number(&now);
    match days {
        0 => format!("Next new wallpaper at {time}"),
        1 => format!("Next new wallpaper tomorrow at {time}"),
        2..=6 => format!("Next new wallpaper on {} at {time}", format(&when, "%A")),
        _ => format!("Next new wallpaper on {} at {time}", format(&when, "%-e %B")),
    }
}

/// A wallpaper's date in History: "Today", "Yesterday", or "5 October 2026".
pub fn day_label(unix: i64) -> String {
    let (Ok(now), Ok(when)) = (glib::DateTime::now_local(), glib::DateTime::from_unix_local(unix)) else {
        return String::new();
    };
    match day_number(&now) - day_number(&when) {
        0 => format!("Today, {}", clock_time(&when)),
        1 => format!("Yesterday, {}", clock_time(&when)),
        _ => format(&when, "%-e %B %Y"),
    }
}

/// "5 October 2026" for an ISO date ("2026-10-05"), as in "Prices as of …".
pub fn iso_date(iso: &str) -> String {
    let mut parts = iso.split('-').filter_map(|part| part.parse::<i32>().ok());
    match (parts.next(), parts.next(), parts.next()) {
        (Some(year), Some(month), Some(day)) => glib::DateTime::from_local(year, month, day, 12, 0, 0.0)
            .map(|date| format(&date, "%-e %B %Y"))
            .unwrap_or_else(|_| iso.to_string()),
        _ => iso.to_string(),
    }
}

/// The time in the person's clock format (GNOME's 12h/24h setting when it can be read; else 24-hour).
fn clock_time(when: &glib::DateTime) -> String {
    let twelve_hour = gtk::gio::SettingsSchemaSource::default()
        .and_then(|source| source.lookup("org.gnome.desktop.interface", true))
        .filter(|schema| schema.has_key("clock-format"))
        .map(|_| gtk::gio::Settings::new("org.gnome.desktop.interface").string("clock-format") == "12h")
        .unwrap_or(false);
    format(when, if twelve_hour { "%-l:%M %p" } else { "%H:%M" })
}

fn format(when: &glib::DateTime, pattern: &str) -> String {
    when.format(pattern).map(|text| text.trim().to_string()).unwrap_or_default()
}

/// Days since an epoch, in local time (for "today" / "tomorrow").
fn day_number(when: &glib::DateTime) -> i64 {
    let (year, month, day) = when.ymd();
    glib::DateTime::from_utc(year, month, day, 0, 0, 0.0).map(|date| date.to_unix() / 86_400).unwrap_or(0)
}

/// Bytes as "380.2 MB" (GLib's own formatting).
pub fn size(bytes: u64) -> String {
    glib::format_size(bytes).to_string()
}

/// The text with its first letter in upper case ("your OpenAI-compatible server" at the start of a sentence).
pub fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_services_explain_the_latest_saved_mood_wallpaper() {
        assert_eq!(stage_label(ProgressStage::CheckingServices), Some("Checking services…"));
        assert_eq!(
            revisit_sentence(RevisitReason::ServicesUnavailable),
            Some("A service is unavailable, so AutoPaper is showing the latest saved wallpaper from this mood. See Console for details.")
        );
        assert_eq!(revisit_sentence(RevisitReason::Requested), None);
    }

    #[test]
    fn console_costs_preserve_paid_subcent_requests() {
        assert_eq!(console_money(3_564), "$0.003564");
        assert_eq!(console_money(1), "$0.000001");
        assert_eq!(console_money(0), "$0.00");
        assert_eq!(console_money(1_250_000), "$1.25");
    }

    #[test]
    fn money_rounds_to_cents() {
        assert_eq!(money(0), "$0.00");
        assert_eq!(money(1_200_000), "$1.20");
        assert_eq!(money(26_000), "$0.03");
        assert_eq!(money(4_999), "under $0.01");
        assert_eq!(money(5_000_000), "$5.00");
        assert_eq!(cents(500), "$5");
        assert_eq!(cents(750), "$7.50");
    }

    #[test]
    fn surprise_speaks_value_and_band() {
        assert_eq!(surprise_spoken(0.35), "35 percent, fresh");
        assert_eq!(surprise_spoken(0.0), "0 percent, faithful");
        assert_eq!(surprise_spoken(0.5), "50 percent, adventurous");
        assert_eq!(surprise_spoken(1.0), "100 percent, wild");
    }

    #[test]
    fn errors_read_as_plain_sentences() {
        let missing = AutoPaperError::MissingKey { provider: ProviderKind::OpenAi };
        assert_eq!(error_sentence(&missing, Fallback::RevisitLiked), "Add your OpenAI key in Preferences to start.");
        let limited = AutoPaperError::RateLimited { provider: ProviderKind::OpenAi, retry_after_secs: 600 };
        assert_eq!(
            error_sentence(&limited, Fallback::RevisitLiked),
            "OpenAI asked AutoPaper to wait; it'll try again in 10 minutes."
        );
        let budget = AutoPaperError::BudgetReached { budget_cents: 500 };
        assert!(error_sentence(&budget, Fallback::RevisitLiked).contains("Your current wallpaper is kept"));
        let compatible = AutoPaperError::Refused { provider: ProviderKind::OpenAiCompatible };
        assert!(error_sentence(&compatible, Fallback::KeepCurrent).starts_with("Your OpenAI-compatible server"));
    }

    #[test]
    fn invalid_input_is_worded_by_reason_never_by_detail() {
        let reasons = [
            InvalidInputReason::KeywordEmpty,
            InvalidInputReason::KeywordTooLong,
            InvalidInputReason::TooManyKeywords,
            InvalidInputReason::DuplicateKeyword,
            InvalidInputReason::AddressMissing,
            InvalidInputReason::AddressNotAllowed,
            InvalidInputReason::AddressInvalid,
            InvalidInputReason::DisplaySizeInvalid,
            InvalidInputReason::NoModels,
            InvalidInputReason::WorkflowNeedsPrompt,
            InvalidInputReason::WorkflowNotApiFormat,
            InvalidInputReason::WorkflowInvalid,
            InvalidInputReason::NothingToEcho,
            InvalidInputReason::Other,
        ];
        for reason in reasons {
            let error = AutoPaperError::InvalidInput { reason, detail: "english detail for logs".into() };
            for text in [error_sentence(&error, Fallback::RevisitLiked), provider_problem(&error)] {
                assert!(!text.contains("english detail"), "{reason:?}: {text}");
                assert!(text.ends_with('.'), "{reason:?}: {text}");
                assert!(!text.contains('!'), "{reason:?}: {text}");
            }
        }
        let long = AutoPaperError::InvalidInput { reason: InvalidInputReason::KeywordTooLong, detail: String::new() };
        assert_eq!(
            error_sentence(&long, Fallback::KeepCurrent),
            format!("Keywords can be up to {} characters.", Keyword::MAX_LEN)
        );
        // Outside Preferences the sentence says where the field is; inside, it doesn't.
        let missing = AutoPaperError::InvalidInput { reason: InvalidInputReason::AddressMissing, detail: String::new() };
        assert_eq!(error_sentence(&missing, Fallback::KeepCurrent), "Enter the server's address in Preferences → Providers.");
        assert_eq!(provider_problem(&missing), "Enter the server's address.");
        assert!(is_address_problem(&missing));
        let workflow = invalid_input(InvalidInputReason::WorkflowNeedsPrompt, Context::Preferences);
        assert!(workflow.contains("{{prompt}}") && !workflow.contains("{{{{"), "{workflow}");
    }

    #[test]
    fn linked_sentences_say_what_to_do_not_where() {
        let missing = AutoPaperError::MissingKey { provider: ProviderKind::OpenAi };
        assert_eq!(linked_sentence(&missing, Fallback::KeepCurrent), "Add your OpenAI key");
        let refused = AutoPaperError::InvalidKey { provider: ProviderKind::Google };
        assert_eq!(linked_sentence(&refused, Fallback::KeepCurrent), "Check your Google Gemini key");
        assert_eq!(key_link(ProviderKind::OpenAiCompatible, false), "Add your server's key");
        let errors = [
            AutoPaperError::InvalidInput { reason: InvalidInputReason::AddressMissing, detail: String::new() },
            AutoPaperError::InvalidInput { reason: InvalidInputReason::AddressInvalid, detail: String::new() },
            AutoPaperError::InvalidInput { reason: InvalidInputReason::AddressNotAllowed, detail: String::new() },
            AutoPaperError::InvalidInput { reason: InvalidInputReason::NoModels, detail: String::new() },
            AutoPaperError::InvalidInput { reason: InvalidInputReason::WorkflowNeedsPrompt, detail: String::new() },
            AutoPaperError::Unsupported { provider: ProviderKind::Ollama, job: "make images".into() },
            AutoPaperError::ProviderUnavailable {
                provider: ProviderKind::ComfyUi,
                reason: ProviderUnavailableReason::NotRunning,
                detail: String::new(),
            },
        ];
        for error in errors {
            let text = linked_sentence(&error, Fallback::KeepCurrent);
            assert!(!text.contains("Preferences"), "{text}");
            assert!(error_sentence(&error, Fallback::KeepCurrent).contains("Preferences"), "{error:?}");
        }
    }

    #[test]
    fn provider_unavailable_is_worded_by_reason() {
        let error = |provider, reason| AutoPaperError::ProviderUnavailable { provider, reason, detail: "HTTP 503".into() };
        let comfy = error(ProviderKind::ComfyUi, ProviderUnavailableReason::NotRunning);
        assert_eq!(
            error_sentence(&comfy, Fallback::KeepCurrent),
            "ComfyUI isn't answering. Check that it's running and its address in Preferences."
        );
        assert_eq!(provider_problem(&comfy), "ComfyUI isn't answering. Is it running, at that address?");
        let busy = error(ProviderKind::OpenAi, ProviderUnavailableReason::ServerError);
        assert_eq!(
            error_sentence(&busy, Fallback::KeepCurrent),
            "OpenAI had a problem on its side. AutoPaper will try again later."
        );
        let slow = error(ProviderKind::OpenAiCompatible, ProviderUnavailableReason::TimedOut);
        assert_eq!(provider_problem(&slow), "Your OpenAI-compatible server took too long to answer. Try again later.");
        let stopped = error(ProviderKind::ComfyUi, ProviderUnavailableReason::Stopped);
        assert_eq!(error_sentence(&stopped, Fallback::KeepCurrent), "The job was stopped in ComfyUI.");
        for text in [&busy, &slow, &stopped].map(|e| error_sentence(e, Fallback::KeepCurrent)) {
            assert!(!text.contains("HTTP"), "{text}");
        }
    }

    #[test]
    fn surprise_bands_are_the_cores() {
        // The thresholds live in the core (`surprise_band`); the words follow its band at every slider step.
        for percent in 0..=100 {
            let value = percent as f32 / 100.0;
            assert_eq!(surprise_band(value).0, format!("{:?}", autopaper_core::surprise_band(value)), "{percent}");
        }
        assert_eq!(surprise_band(f32::NAN).0, "Faithful");
    }

    #[test]
    fn default_model_choice_names_the_cores_default() {
        assert_eq!(
            default_model_label(ProviderKind::OpenAi, ProviderJob::Concepts, false),
            format!("Default ({})", default_model(ProviderKind::OpenAi, ProviderJob::Concepts))
        );
        assert!(!default_model(ProviderKind::OpenAi, ProviderJob::Concepts).is_empty());
        assert_eq!(default_model_label(ProviderKind::ComfyUi, ProviderJob::Images, false), "Default (Z-Image Turbo)");
        assert_eq!(default_model_label(ProviderKind::ComfyUi, ProviderJob::Images, true), "Default (your workflow's model)");
        assert_eq!(default_model_label(ProviderKind::Ollama, ProviderJob::Concepts, false), "Default (the server's first model)");
        assert_eq!(
            default_model_label(ProviderKind::OpenAiCompatible, ProviderJob::Images, false),
            "Default (the server's first model)"
        );
    }

    #[test]
    fn painting_failures_are_one_line_naming_the_model() {
        let failed = AutoPaperError::PaintingFailed {
            provider: ProviderKind::ComfyUi,
            model: "Qwen-Image 2.1".into(),
            detail: "the KSampler node (8) failed: english detail".into(),
        };
        assert_eq!(linked_sentence(&failed, Fallback::KeepCurrent), "ComfyUI couldn't paint with Qwen-Image 2.1.");
        assert_eq!(provider_problem(&failed), "ComfyUI couldn't paint with Qwen-Image 2.1.");
        let toast = error_sentence(&failed, Fallback::KeepCurrent);
        assert!(toast.starts_with("ComfyUI couldn't paint with Qwen-Image 2.1.") && toast.contains("Providers"), "{toast}");
        for text in [toast, provider_problem(&failed)] {
            assert!(!text.contains("KSampler") && !text.contains("try again"), "{text}");
        }
        let unnamed = AutoPaperError::PaintingFailed { provider: ProviderKind::ComfyUi, model: " ".into(), detail: String::new() };
        assert_eq!(linked_sentence(&unnamed, Fallback::KeepCurrent), "ComfyUI couldn't paint this wallpaper.");
        assert_eq!(painting_link(ProviderKind::ComfyUi), "Check ComfyUI's settings");
    }

    #[test]
    fn mood_reasons_have_their_own_sentences() {
        for reason in [
            InvalidInputReason::MoodNameEmpty,
            InvalidInputReason::MoodNameTooLong,
            InvalidInputReason::DuplicateMoodName,
            InvalidInputReason::LastMood,
        ] {
            let error = AutoPaperError::InvalidInput { reason, detail: "english detail for logs".into() };
            let text = error_sentence(&error, Fallback::KeepCurrent);
            assert!(text.to_lowercase().contains("mood") && text.ends_with('.') && !text.contains("english"), "{reason:?}: {text}");
            assert_ne!(text, "Something went wrong inside AutoPaper.");
        }
        assert_eq!(invalid_input(InvalidInputReason::MoodNameTooLong, Context::Main), "Mood names can be up to 40 characters.");
    }

    fn mood(name: &str, active: bool) -> Mood {
        Mood {
            id: name.to_lowercase(),
            name: name.into(),
            position: 0,
            surprise: 0.35,
            created_at: 0,
            keywords: Vec::new(),
            active,
        }
    }

    #[test]
    fn new_and_copied_moods_get_unused_names() {
        let moods = vec![mood("Rainy beach", true), mood("new MOOD", false), mood("New mood 2", false)];
        assert_eq!(unused_mood_name("New mood", &moods), "New mood 3");
        assert_eq!(unused_mood_name("Forest", &moods), "Forest");
        assert_eq!(mood_copy_name("Rainy beach", &moods), "Rainy beach copy");
        let with_copy = [moods.clone(), vec![mood("Rainy beach copy", false)]].concat();
        assert_eq!(mood_copy_name("Rainy beach", &with_copy), "Rainy beach copy 2");
        // Within the core's 40 characters, also when numbered.
        let long = "A".repeat(40);
        let taken = vec![mood(&long, false)];
        let name = mood_copy_name(&long, &taken);
        assert!(name.chars().count() <= Mood::MAX_NAME_LEN, "{name}");
        let numbered = unused_mood_name(&long, &taken);
        assert!(numbered.ends_with(" 2") && numbered.chars().count() <= Mood::MAX_NAME_LEN, "{numbered}");
    }

    #[test]
    fn moods_read_in_short_and_aloud() {
        let keyword = |text: &str, weight| Keyword { id: text.into(), text: text.into(), weight, position: 0, created_at: 0 };
        let keywords = [
            keyword("rain", KeywordWeight::Must),
            keyword("beach", KeywordWeight::Maybe),
            keyword("people", KeywordWeight::Avoid),
        ];
        assert_eq!(mood_keywords(&keywords), "rain · beach · no people");
        assert_eq!(mood_keywords(&[]), "No keywords yet");
        assert_eq!(mood_spoken(&mood("Rainy beach", true)), "Rainy beach, current mood");
        assert_eq!(mood_spoken(&mood("Forest", false)), "Forest");
    }

    #[test]
    fn time_left_is_rounded_like_the_estimate_behind_it() {
        assert_eq!(time_left(0), "a few seconds left");
        assert_eq!(time_left(10), "a few seconds left");
        assert_eq!(time_left(11), "about 15 seconds left");
        assert_eq!(time_left(42), "about 45 seconds left");
        assert_eq!(time_left(75), "about 1 minute left");
        assert_eq!(time_left(360), "about 6 minutes left");
        assert_eq!(time_left(7200), "about 2 hours left");
        assert_eq!(progress_spoken(0.456, Some(130)), "45 percent, about 2 minutes left");
        assert_eq!(progress_spoken(1.4, None), "100 percent");
        assert_eq!(
            estimate_line(ProviderJob::Images, ProviderKind::ComfyUi, 540),
            "About 9 minutes per wallpaper on this computer"
        );
        assert_eq!(estimate_line(ProviderJob::Concepts, ProviderKind::OpenAi, 4), "About 4 seconds per idea");
        assert_eq!(duration(1), "1 second");
    }

    #[test]
    fn a_keyword_not_followed_is_one_line_naming_it() {
        let missed = AutoPaperError::KeywordNotFollowed {
            keyword: "lighthouse".into(),
            weight: KeywordWeight::Must,
            mood_id: "m1".into(),
        };
        let avoided = AutoPaperError::KeywordNotFollowed {
            keyword: "people".into(),
            weight: KeywordWeight::Avoid,
            mood_id: "m1".into(),
        };
        assert_eq!(linked_sentence(&missed, Fallback::KeepCurrent), "The writing model kept leaving out “lighthouse”.");
        assert_eq!(
            linked_sentence(&avoided, Fallback::KeepCurrent),
            "The writing model kept including “people”, which this mood avoids."
        );
        // The same one line wherever it shows; never "try again" (asking again rarely helps).
        for error in [&missed, &avoided] {
            for text in [error_sentence(error, Fallback::RevisitLiked), provider_problem(error)] {
                assert!(!text.contains("try again") && !text.contains("Preferences"), "{text}");
            }
        }
    }

    fn generation(text: (ProviderKind, &str), image: (ProviderKind, &str), cost: u64) -> autopaper_core::Generation {
        autopaper_core::Generation {
            id: "g".into(),
            created_at: 0,
            trigger: autopaper_core::Trigger::Manual,
            status: autopaper_core::GenerationStatus::Ok,
            concept: autopaper_core::Concept::default(),
            image_path: None,
            thumb_path: None,
            width: 3840,
            height: 2160,
            rating: Rating::Unrated,
            echo_of: None,
            echo_note: None,
            text_provider: text.0,
            text_model: text.1.into(),
            image_provider: image.0,
            image_model: image.1.into(),
            surprise: 0.35,
            keywords: Vec::new(),
            cost_microusd: cost,
            last_shown_at: None,
            shown_count: 0,
            error: None,
            mood_id: None,
            mood_name: None,
        }
    }

    #[test]
    fn provenance_names_the_models_in_words() {
        let hosted = generation((ProviderKind::Google, "gemini-3.5-flash-lite"), (ProviderKind::Google, "gemini-3.1-flash-image"), 40_000);
        let line = Provenance::of(&hosted);
        assert_eq!(
            line.text(),
            "Written by Gemini 3.5 Flash-Lite (Google Gemini) · Painted by Nano Banana 2 (Google Gemini) · 3840×2160 · about $0.04"
        );
        assert_eq!(
            line.spoken(),
            "Written by Gemini 3.5 Flash-Lite (Google Gemini), Painted by Nano Banana 2 (Google Gemini), 3840 by 2160, about $0.04"
        );
        assert_eq!(line.tail(), " · 3840×2160 · about $0.04");
        // Local and Demo cost nothing; ComfyUI files read as their names; Demo is just "Demo".
        let local = generation((ProviderKind::Demo, ""), (ProviderKind::ComfyUi, "models/My Model v2.safetensors"), 0);
        assert_eq!(Provenance::of(&local).text(), "Written by Demo · Painted by My Model v2 (ComfyUI) · 3840×2160");
        let bundled = generation((ProviderKind::Ollama, "llama4"), (ProviderKind::ComfyUi, "z_image_turbo_bf16.safetensors"), 0);
        assert_eq!(Provenance::of(&bundled).painted, "Painted by Z-Image Turbo (ComfyUI)");
        assert_eq!(Provenance::of(&bundled).written, "Written by llama4 (Ollama)");
    }

    #[test]
    fn moods_are_summed_up_in_words() {
        assert_eq!(surprise_line(0.35), "Surprise: Fresh (35%)");
        assert_eq!(surprise_line(f32::NAN), "Surprise: Faithful (0%)");
        let now = glib::DateTime::from_local(2026, 10, 6, 15, 0, 0.0).unwrap().to_unix();
        assert_eq!(relative_time(now - 20, now), "just now");
        assert_eq!(relative_time(now + 500, now), "just now");
        assert_eq!(relative_time(now - 60, now), "1 minute ago");
        assert_eq!(relative_time(now - 3 * 3600, now), "3 hours ago");
        assert_eq!(relative_time(now - 24 * 3600, now), "yesterday");
        assert_eq!(relative_time(now - 3 * 86_400, now), "3 days ago");
        assert_eq!(relative_time(now - 15 * 86_400, now), "2 weeks ago");
        assert_eq!(relative_time(now - 400 * 86_400, now), "1 year ago");
        assert_eq!(made_line(0, 0, None, " · ", now), "Nothing made yet");
        assert_eq!(made_line(1, 0, Some(now - 3 * 3600), " · ", now), "1 wallpaper · 0 liked · last made 3 hours ago");
        assert_eq!(made_line(12, 3, None, ", ", now), "12 wallpapers, 3 liked");
        assert_eq!(last_made_tile(None, now), "Not yet");
        assert_eq!(last_made_tile(Some(now - 86_400), now), "Yesterday");
        let keyword = |text: &str, weight| Keyword { id: text.into(), text: text.into(), weight, position: 0, created_at: 0 };
        let mut rainy = mood("Rainy", true);
        rainy.keywords = vec![
            keyword("rain", KeywordWeight::Must),
            keyword("people", KeywordWeight::Avoid),
            keyword("beach", KeywordWeight::Must),
        ];
        assert_eq!(
            keyword_groups(rainy.keywords.iter().map(|k| (k.text.as_str(), k.weight))),
            vec!["Must: rain, beach".to_string(), "Avoid: people".to_string()]
        );
        assert_eq!(keyword_breakdown(&[rainy.clone(), mood("Empty", false)]), "2 Must · 0 Maybe · 1 Avoid keywords");
        assert_eq!(keyword_breakdown(&[mood("Empty", false)]), "No keywords yet");
        assert_eq!(spoken_latest(&["Fog".into()]), "Latest wallpaper: Fog");
        assert_eq!(spoken_latest(&["Fog".into(), "Harbour".into()]), "Latest wallpapers: Fog, Harbour");
    }

    #[test]
    fn rating_joins_the_description() {
        assert_eq!(spoken_description("Rain. Ruins at night.", Rating::Liked), "Rain. Ruins at night. Liked.");
        assert_eq!(spoken_description("Rain.", Rating::Unrated), "Rain.");
    }
}
