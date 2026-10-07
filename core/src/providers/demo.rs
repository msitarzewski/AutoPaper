//! Demo provider: no network, no key, no cost. Composes plausible concepts from the keywords with a
//! seeded RNG (setting/time/weather/palette vocabularies, honouring Must/Maybe/Avoid and Surprise's
//! wildcard limit) and paints a soft gradient PNG from the concept's palette words. Used by tests,
//! `autopaper simulate`, and for trying the apps before adding a key.
//!
//! Concepts come from `ComposeRequest::inputs` (never by parsing the instructions): every Must is in each
//! candidate's prompt and `keywords_used`; some Maybes are; nothing the Demo writes mentions an Avoid
//! (checked with `text::mentions`); wildcards stay within Surprise's limit. For an echo the original's
//! setting is kept (unless it now mentions an Avoid), Musts don't apply, and each candidate has an
//! `echo_note`. Text is English whatever the locale, with "a"/"an" by sound ("an oil painting") and each thing
//! named once (a keyword the setting, subject, time, weather or season already names isn't listed again). The
//! same seed and the same sequence of requests give the same concepts; images depend only on the request
//! (prompt, size, seed).
//!
//! **Slow mode, for testing progress UI** (`with_delay`; the engine sets it from `AUTOPAPER_DEMO_DELAY_SECS` when
//! it opens, off by default): writing takes a quarter of the delay and painting the rest, reported as
//! `DEMO_STEPS` steps through `ImageRequest::progress`, like ComfyUI's sampler — so hosts can show a determinate
//! ring and time left in a VM without ComfyUI. A cancel drops the wait at once.

use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use rand::Rng;
use rand::rngs::StdRng;
use rand::seq::IndexedRandom;

use crate::error::{AutoPaperError, InvalidInputReason, Result};
use crate::model::{Concept, ModelInfo, ProviderKind};
use crate::text;

use super::{
    ComposeInputs, ComposeRequest, ComposeResponse, FreeSize, ImageCapabilities, ImageProvider, ImageRequest, ImageResponse, PaintStep,
    TextProvider, Usage,
};

pub(crate) const MODEL: &str = "demo";
/// Steps a slow Demo painting reports.
pub const DEMO_STEPS: u32 = 8;
/// The longest delay slow mode takes (an hour).
pub const MAX_DELAY: Duration = Duration::from_secs(60 * 60);
/// Largest image painted, landscape (portrait is the transpose).
const MAX_WIDTH: u32 = 3840;
const MAX_HEIGHT: u32 = 2160;
/// Candidates written when the request doesn't say, and the most written per call.
const MAX_CANDIDATES: usize = 16;
const TITLE_MAX_CHARS: usize = 60;

pub struct Demo {
    rng: Mutex<StdRng>,
    /// Slow mode: how long a wallpaper takes (writing + painting); zero = as fast as it can.
    delay: Duration,
}

impl Demo {
    pub fn new(seed: u64) -> Self {
        use rand::SeedableRng;
        Self { rng: Mutex::new(StdRng::seed_from_u64(seed)), delay: Duration::ZERO }
    }

    /// Slow mode (see the module docs): a wallpaper takes about `delay` (at most `MAX_DELAY`).
    pub fn with_delay(mut self, delay: Duration) -> Self {
        self.delay = delay.min(MAX_DELAY);
        self
    }

    fn slow(&self) -> bool {
        !self.delay.is_zero()
    }
}

#[async_trait]
impl TextProvider for Demo {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Demo
    }

    fn default_model(&self) -> &str {
        MODEL
    }

    /// Writes `{"candidates": [...]}` matching `composer::schema` (with `echo_note` for echoes) from
    /// `request.inputs`.
    async fn compose(&self, request: ComposeRequest) -> Result<ComposeResponse> {
        if self.slow() {
            tokio::time::sleep(self.delay / 4).await;
        }
        let inputs = &request.inputs;
        let count = match inputs.candidates {
            0 => crate::composer::CANDIDATES,
            n => n.min(MAX_CANDIDATES),
        };
        let candidates = {
            let mut rng = self.rng.lock().unwrap_or_else(PoisonError::into_inner);
            (0..count).map(|_| write_candidate(inputs, &mut *rng)).collect::<Vec<_>>()
        };
        let mut output = Vec::with_capacity(candidates.len());
        for (concept, echo_note) in candidates {
            let mut value = serde_json::to_value(concept)?;
            if let (Some(note), Some(object)) = (echo_note, value.as_object_mut()) {
                object.insert("echo_note".into(), serde_json::Value::String(note));
            }
            output.push(value);
        }
        Ok(ComposeResponse { output: serde_json::json!({ "candidates": output }), usage: Usage::default(), model: MODEL.into() })
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        Ok(vec![ModelInfo { id: "demo".into(), display_name: "Demo".into() }])
    }
}

#[async_trait]
impl ImageProvider for Demo {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Demo
    }

    fn default_model(&self) -> &str {
        MODEL
    }

    /// Any size in steps of 8 up to 3840 × 2160.
    fn capabilities(&self, _model: &str) -> ImageCapabilities {
        ImageCapabilities {
            sizes: Vec::new(),
            free_size: Some(FreeSize { step: 8, max_side: MAX_WIDTH, max_pixels: u64::from(MAX_WIDTH) * u64::from(MAX_HEIGHT) }),
        }
    }

    /// `DEMO_STEPS` in slow mode; nothing to report otherwise.
    fn steps(&self, _model: &str) -> Option<u32> {
        self.slow().then_some(DEMO_STEPS)
    }

    /// A soft vertical gradient with a gentle radial glow, coloured from the colour words in the prompt
    /// (else colours derived from the prompt's hash), at the requested size scaled down to fit 3840 × 2160.
    /// In slow mode, three quarters of the delay first, in `DEMO_STEPS` reported steps.
    async fn generate(&self, request: ImageRequest) -> Result<ImageResponse> {
        if request.width == 0 || request.height == 0 {
            return Err(AutoPaperError::invalid_input(InvalidInputReason::Other, "the image size must be at least 1 × 1"));
        }
        if self.slow() {
            let step = (self.delay - self.delay / 4) / DEMO_STEPS;
            for done in 1..=DEMO_STEPS {
                tokio::time::sleep(step).await;
                if let Some(progress) = &request.progress {
                    progress.report(PaintStep { done, total: DEMO_STEPS });
                }
            }
        }
        let model = match request.model.trim() {
            "" => MODEL.to_string(),
            named => named.to_string(),
        };
        let bytes = tokio::task::spawn_blocking(move || paint(&request.prompt, request.width, request.height, request.seed))
            .await
            .map_err(|error| AutoPaperError::Internal { detail: format!("painting the demo image failed: {error}") })??;
        Ok(ImageResponse { bytes, mime: "image/png".into(), model, reported_cost_microusd: None })
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        Ok(vec![ModelInfo { id: "demo".into(), display_name: "Demo".into() }])
    }
}

// ── Composing ─────────────────────────────────────────────────────────────────────────────────

const SETTINGS: &[&str] = &[
    "a quiet alpine lake",
    "coastal cliffs",
    "a desert canyon",
    "rolling hills",
    "a birch forest",
    "a mountain valley",
    "salt flats",
    "a river delta",
    "a terraced garden",
    "a harbour town",
    "an island archipelago",
    "a high plateau",
    "a deep fjord",
    "a wildflower meadow",
    "a frozen tundra",
    "a bamboo grove",
    "volcanic highlands",
    "a sandstone arch",
];
const SUBJECTS: &[&str] = &[
    "a winding path",
    "a lone tree",
    "distant peaks",
    "a still reflection",
    "a stone bridge",
    "a lighthouse",
    "drifting clouds",
    "an old observatory",
    "a circle of standing stones",
    "a wooden pier",
    "a glasshouse",
    "a cascading waterfall",
];
const ELEMENTS: &[&str] = &[
    "low mist",
    "scattered boulders",
    "tall grass",
    "wet stones",
    "a narrow stream",
    "pine silhouettes",
    "soft dunes",
    "reed beds",
    "lichen-covered rocks",
    "a gravel shore",
    "fallen leaves",
    "a field of ferns",
];
const TIMES: &[&str] = &["dawn", "early morning", "midday", "late afternoon", "golden hour", "dusk", "blue hour", "night"];
const WEATHER: &[&str] = &["clear skies", "light rain", "fog", "falling snow", "scattered clouds", "a passing storm", "haze", "still air"];
const SEASONS: &[&str] = &["spring", "summer", "autumn", "winter"];
const MOODS: &[&str] =
    &["calm", "serene", "wistful", "hopeful", "mysterious", "contemplative", "dreamy", "quiet", "luminous", "melancholic"];
const SHADES: &[&str] = &["soft", "deep", "pale", "muted", "warm", "dusty"];
const STYLES: &[&str] = &[
    "soft matte painting",
    "atmospheric photograph",
    "watercolour illustration",
    "minimal vector illustration",
    "oil painting",
    "cinematic photograph",
    "gouache illustration",
    "long-exposure photograph",
];
const COMPOSITIONS: &[&str] = &[
    "a wide panorama with open sky above",
    "a low horizon with a quiet foreground",
    "a rule-of-thirds landscape with calm space on the left",
    "a centred vista with an uncluttered sky",
    "layered depth from foreground to a distant horizon",
];
const WILDCARDS: &[&str] = &[
    "a single red kite in the sky",
    "floating paper lanterns",
    "a forgotten bicycle",
    "a flock of white cranes",
    "a sunken bell",
    "a lone telescope",
    "a ribbon of aurora",
    "an overgrown carousel",
    "a drifting hot-air balloon",
    "a whale breaching far away",
    "a field of glass flowers",
    "an abandoned greenhouse",
];
const CONNECTORS: &[&str] = &["over", "at", "beyond", "across"];

/// Wildcards allowed: the composer's rule (0 below 0.25, ≤1 below 0.5, ≤2 below 0.75, else ≤3).
use crate::composer::wildcard_limit;

/// Mentions no Avoid.
fn allowed(phrase: &str, avoids: &[String]) -> bool {
    !avoids.iter().any(|avoid| text::mentions(phrase, avoid))
}

fn pick<'a, R: Rng + ?Sized>(rng: &mut R, list: &[&'a str], avoids: &[String], exclude: &[&str]) -> Option<&'a str> {
    let options: Vec<&str> = list.iter().copied().filter(|item| allowed(item, avoids) && !exclude.contains(item)).collect();
    options.choose(rng).copied()
}

fn pick_many<'a, R: Rng + ?Sized>(rng: &mut R, list: &[&'a str], avoids: &[String], amount: usize) -> Vec<&'a str> {
    let options: Vec<&str> = list.iter().copied().filter(|item| allowed(item, avoids)).collect();
    options.choose_multiple(rng, amount).copied().collect()
}

/// The vocabulary item a chosen keyword names (e.g. "rain" → "light rain", "night sky" → "night"), if any.
fn named_by<'a>(list: &[&'a str], keywords: &[String], avoids: &[String]) -> Option<&'a str> {
    list.iter()
        .copied()
        .find(|item| allowed(item, avoids) && keywords.iter().any(|kw| text::mentions(item, kw) || text::mentions(kw, item)))
}

/// "a" or "an" before `phrase`, by the sound of its first word: "an" before a vowel letter ("an oil painting",
/// "an atmospheric photograph") or a silent h ("an hour"), "a" otherwise, including vowels that sound like "you"
/// or "w" ("a unique", "a European", "a one-off").
fn indefinite(phrase: &str) -> &'static str {
    const SILENT_H: [&str; 4] = ["hour", "honest", "honour", "heir"];
    const CONSONANT_SOUNDING: [&str; 7] = ["uni", "use", "usu", "uti", "eu", "ewe", "one"];
    let word = phrase.trim_start().to_lowercase();
    if SILENT_H.iter().any(|prefix| word.starts_with(prefix)) {
        "an"
    } else if CONSONANT_SOUNDING.iter().any(|prefix| word.starts_with(prefix)) {
        "a"
    } else if word.starts_with(['a', 'e', 'i', 'o', 'u']) {
        "an"
    } else {
        "a"
    }
}

/// `parts` with each thing named once: a part that `said` (the setting, time, weather, season) or a part
/// already kept names is dropped ("lighthouse" after "a lighthouse", "canyon" with "a desert canyon"), and a
/// fuller part replaces a kept one it names ("lichen-covered rocks" replaces "rocks", in its place). A part that
/// was dropped is still in the text through the part that names it, so every Must stays mentioned.
fn distinct_parts(parts: impl IntoIterator<Item = String>, said: &[&str]) -> Vec<String> {
    let mut kept: Vec<String> = Vec::new();
    for part in parts {
        let named = |by: &str| text::mentions(by, &part);
        if part.trim().is_empty() || said.iter().any(|by| named(by)) || kept.iter().any(|by| named(by)) {
            continue;
        }
        match kept.iter().position(|shorter| text::mentions(&part, shorter)) {
            Some(first) => {
                let mut index = 0;
                kept.retain(|shorter| {
                    let keep = index == first || !text::mentions(&part, shorter);
                    index += 1;
                    keep
                });
                kept[first] = part;
            }
            None => kept.push(part),
        }
    }
    kept
}

fn without_article(phrase: &str) -> &str {
    ["a ", "an ", "the "].iter().find_map(|article| phrase.strip_prefix(article)).unwrap_or(phrase)
}

fn capitalise(phrase: &str) -> String {
    let mut chars = phrase.chars();
    chars.next().map(|first| first.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

fn title_case(phrase: &str) -> String {
    phrase.split(' ').map(capitalise).collect::<Vec<_>>().join(" ")
}

/// "a", "a and b", "a, b and c".
fn list_phrase(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
}

/// Text that mentions no Avoid: `preferred` if it's clean, else the bare `parts` joined with commas,
/// dropping optional parts from the end until clean (required parts — the person's Musts — always stay).
fn clean_text(preferred: String, parts: &[(String, bool)], avoids: &[String]) -> String {
    if allowed(&preferred, avoids) {
        return preferred;
    }
    let mut kept: Vec<&(String, bool)> = parts.iter().filter(|(part, _)| !part.is_empty()).collect();
    loop {
        let joined = kept.iter().map(|(part, _)| part.as_str()).collect::<Vec<_>>().join(", ");
        if allowed(&joined, avoids) {
            return capitalise(&joined);
        }
        match kept.iter().rposition(|(_, required)| !required) {
            Some(index) => {
                kept.remove(index);
            }
            // Only Musts are left, and one of them mentions an Avoid: the person's own contradiction.
            None => return capitalise(&joined),
        }
    }
}

fn cleaned(list: &[String], avoids: &[String]) -> Vec<String> {
    list.iter().map(|item| item.trim()).filter(|item| !item.is_empty() && allowed(item, avoids)).map(String::from).collect()
}

/// One candidate concept and, for echoes, its note.
fn write_candidate<R: Rng + ?Sized>(inputs: &ComposeInputs, rng: &mut R) -> (Concept, Option<String>) {
    let avoids: Vec<String> = inputs.avoids.iter().map(|a| a.trim().to_string()).filter(|a| !a.is_empty()).collect();
    let surprise = if inputs.surprise.is_nan() { 0.0 } else { inputs.surprise.clamp(0.0, 1.0) };
    let original = inputs.echo_of.as_ref();

    // Keywords: all Musts (never for echoes — it's the old idea's echo), some Maybes.
    let (musts, maybes) = match original {
        Some(original) => (Vec::new(), cleaned(&original.keywords_used, &avoids)),
        None => {
            let musts: Vec<String> = inputs.musts.iter().map(|m| m.trim().to_string()).filter(|m| !m.is_empty()).collect();
            let maybes = cleaned(&inputs.maybes, &avoids);
            let some = match maybes.len() {
                0 => 0,
                n => rng.random_range(1..=n.div_ceil(2)),
            };
            let chosen: Vec<String> = maybes.choose_multiple(rng, some).cloned().collect();
            (musts, chosen)
        }
    };
    let keywords: Vec<String> = musts.iter().chain(&maybes).cloned().collect();

    let limit = wildcard_limit(surprise);
    let wildcard_count = rng.random_range(limit.saturating_sub(1)..=limit);
    let wildcards: Vec<String> = pick_many(rng, WILDCARDS, &avoids, wildcard_count).into_iter().map(String::from).collect();

    let kept_setting = original.map(|o| o.setting.trim()).filter(|s| !s.is_empty() && allowed(s, &avoids));
    let setting = kept_setting.map(String::from).or_else(|| pick(rng, SETTINGS, &avoids, &[]).map(String::from)).unwrap_or_default();
    let kept_subject = original.map(|o| o.subject.trim()).filter(|s| !s.is_empty() && allowed(s, &avoids));
    let subject = kept_subject.map(String::from).or_else(|| pick(rng, SUBJECTS, &avoids, &[]).map(String::from)).unwrap_or_default();

    // An echo moves to another time, weather and season; otherwise a keyword may name them.
    let previous = |field: fn(&Concept) -> &str| original.map(field).map(str::trim).into_iter().collect::<Vec<_>>();
    let choose = |rng: &mut R, list: &[&str], previous: &[&str]| {
        let named = if original.is_none() { named_by(list, &keywords, &avoids) } else { None };
        named.or_else(|| pick(rng, list, &avoids, previous)).map(String::from).unwrap_or_default()
    };
    let time_of_day = choose(rng, TIMES, &previous(|c| &c.time_of_day));
    let weather = choose(rng, WEATHER, &previous(|c| &c.weather));
    let season = choose(rng, SEASONS, &previous(|c| &c.season));

    let filler = if surprise < 0.25 { 1 } else { 2 };
    let mut elements: Vec<String> = match original {
        Some(original) => cleaned(&original.elements, &avoids).into_iter().take(4).collect(),
        None => Vec::new(),
    };
    // A keyword that became the time, weather or season ("rain" → "light rain") isn't repeated.
    let named_already = |keyword: &String| [&time_of_day, &weather, &season].iter().any(|field| text::mentions(field, keyword));
    elements.extend(keywords.iter().filter(|keyword| !named_already(keyword)).cloned());
    elements.extend(pick_many(rng, ELEMENTS, &avoids, filler).into_iter().map(String::from));
    let mut seen = std::collections::HashSet::new();
    elements.retain(|element| seen.insert(element.to_lowercase()));

    let mood: Vec<String> = pick_many(rng, MOODS, &avoids, 2).into_iter().map(String::from).collect();
    let palette: Vec<String> = pick_many(rng, PALETTE_NAMES, &avoids, 3)
        .into_iter()
        .map(|colour| match pick(rng, SHADES, &avoids, &[]) {
            Some(shade) if allowed(&format!("{shade} {colour}"), &avoids) => format!("{shade} {colour}"),
            _ => colour.to_string(),
        })
        .collect();
    let style = pick(rng, STYLES, &avoids, &[]).map(String::from).unwrap_or_default();
    let composition = pick(rng, COMPOSITIONS, &avoids, &[]).map(String::from).unwrap_or_default();

    // The prompt names every Must (verbatim, or inside the time/weather/season it named), so
    // `text::mentions` finds each one; the bare fallback keeps them verbatim.
    let scene = distinct_parts(
        std::iter::once(subject.clone()).chain(elements.iter().cloned()).chain(wildcards.iter().cloned()),
        &[&setting, &time_of_day, &weather, &season],
    );
    let moods = list_phrase(&mood);
    let preferred_prompt = format!(
        "{} of {setting} at {time_of_day}, {weather}, in {season}: {}. {} {moods} mood, a palette of {}. {}, a wide full-bleed scene.",
        capitalise(&style),
        list_phrase(&scene),
        capitalise(indefinite(&moods)),
        list_phrase(&palette),
        capitalise(&composition),
    );
    let mut parts: Vec<(String, bool)> = vec![(style.clone(), false), (setting.clone(), false)];
    parts.extend(musts.iter().map(|must| (must.clone(), true)));
    parts.extend(
        [time_of_day.clone(), weather.clone(), season.clone(), subject.clone()]
            .into_iter()
            .chain(elements.iter().filter(|e| !musts.contains(e)).cloned())
            .chain(wildcards.iter().cloned())
            .chain(palette.iter().cloned())
            .chain(mood.iter().cloned())
            .chain(std::iter::once(composition.clone()))
            .map(|part| (part, false)),
    );
    let prompt = clean_text(preferred_prompt, &parts, &avoids);

    let lead = keywords.first().cloned().unwrap_or_else(|| time_of_day.clone());
    let connector = pick(rng, CONNECTORS, &avoids, &[]).unwrap_or("at");
    let place = title_case(without_article(&setting));
    let title =
        clean_text(format!("{} {connector} {place}", capitalise(&lead)), &[(place.clone(), false), (capitalise(&lead), false)], &avoids);
    let title = truncate_words(&title, TITLE_MAX_CHARS);

    let preferred_summary = format!(
        "{} {style} of {setting} at {time_of_day} in {season}, {weather}, with {}. Its palette is {}, and the mood {}.",
        capitalise(indefinite(&style)),
        list_phrase(&scene),
        list_phrase(&palette),
        list_phrase(&mood),
    );
    let summary_parts: Vec<(String, bool)> = [setting.clone(), time_of_day.clone(), weather.clone(), subject.clone()]
        .into_iter()
        .map(|part| (part, false))
        .chain(musts.iter().map(|must| (must.clone(), true)))
        .collect();
    let summary = clean_text(preferred_summary, &summary_parts, &avoids);

    let echo_note = original.map(|original| {
        let echoed = match original.title.trim() {
            title if !title.is_empty() && allowed(title, &avoids) => format!("Echo of “{title}”"),
            _ => "Echo of an earlier image".to_string(),
        };
        let change: Vec<String> = [&time_of_day, &weather].into_iter().filter(|p| !p.is_empty()).cloned().collect();
        let preferred = match change.as_slice() {
            [] => format!("{echoed}."),
            change => format!("{echoed}: now {}.", change.join(", ")),
        };
        let parts: Vec<(String, bool)> = change.into_iter().map(|part| (part, false)).collect();
        clean_text(preferred, &parts, &avoids)
    });

    let concept = Concept {
        title,
        summary,
        setting,
        subject,
        elements,
        time_of_day,
        weather,
        season,
        mood,
        palette,
        style,
        composition,
        keywords_used: keywords,
        wildcards,
        prompt,
    };
    (concept, echo_note)
}

/// At most `max` characters, cut at a word boundary.
fn truncate_words(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out = String::new();
    for word in text.split(' ') {
        let next = if out.is_empty() { word.chars().count() } else { out.chars().count() + 1 + word.chars().count() };
        if next > max {
            break;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    if out.is_empty() { text.chars().take(max).collect() } else { out }
}

// ── Painting ──────────────────────────────────────────────────────────────────────────────────

/// Colour words the painter knows, and the palette vocabulary.
const COLOURS: &[(&str, [u8; 3])] = &[
    ("black", [18, 18, 22]),
    ("charcoal", [54, 58, 64]),
    ("slate", [88, 100, 116]),
    ("grey", [128, 132, 138]),
    ("gray", [128, 132, 138]),
    ("silver", [190, 196, 204]),
    ("white", [240, 240, 236]),
    ("ivory", [242, 236, 218]),
    ("cream", [238, 228, 200]),
    ("sand", [214, 192, 150]),
    ("peach", [246, 190, 160]),
    ("coral", [240, 128, 110]),
    ("pink", [236, 160, 180]),
    ("rose", [214, 112, 138]),
    ("red", [190, 48, 48]),
    ("crimson", [160, 24, 48]),
    ("rust", [168, 82, 46]),
    ("copper", [184, 115, 51]),
    ("orange", [232, 132, 48]),
    ("amber", [230, 160, 40]),
    ("gold", [214, 176, 72]),
    ("golden", [214, 176, 72]),
    ("yellow", [236, 210, 90]),
    ("ochre", [196, 150, 60]),
    ("olive", [120, 124, 60]),
    ("moss", [104, 124, 70]),
    ("sage", [150, 170, 140]),
    ("green", [70, 140, 90]),
    ("emerald", [30, 130, 96]),
    ("mint", [160, 220, 190]),
    ("teal", [30, 120, 128]),
    ("turquoise", [64, 190, 190]),
    ("cyan", [80, 190, 220]),
    ("blue", [50, 96, 180]),
    ("cobalt", [36, 72, 170]),
    ("navy", [24, 36, 82]),
    ("indigo", [56, 48, 120]),
    ("violet", [120, 80, 170]),
    ("purple", [110, 60, 140]),
    ("lavender", [176, 160, 214]),
    ("magenta", [190, 60, 150]),
    ("brown", [110, 78, 52]),
];
const PALETTE_NAMES: &[&str] = &[
    "charcoal",
    "slate",
    "silver",
    "ivory",
    "sand",
    "peach",
    "coral",
    "rose",
    "crimson",
    "rust",
    "copper",
    "amber",
    "gold",
    "ochre",
    "olive",
    "moss",
    "sage",
    "emerald",
    "teal",
    "turquoise",
    "cobalt",
    "navy",
    "indigo",
    "violet",
    "lavender",
];

fn colour_named(word: &str) -> Option<[u8; 3]> {
    COLOURS.iter().find(|(name, _)| *name == word).map(|(_, rgb)| *rgb)
}

/// Up to three distinct colours named in the prompt, in the order they appear.
fn palette_colours(prompt: &str) -> Vec<[f32; 3]> {
    let mut found: Vec<[u8; 3]> = Vec::new();
    for word in text::words(prompt) {
        let colour = colour_named(&word).or_else(|| colour_named(&text::stem(&word)));
        if let Some(colour) = colour.filter(|c| !found.contains(c)) {
            found.push(colour);
            if found.len() == 3 {
                break;
            }
        }
    }
    found.into_iter().map(|[r, g, b]| [f32::from(r), f32::from(g), f32::from(b)]).collect()
}

/// FNV-1a, stable across platforms and releases (unlike `std`'s hasher).
fn fnv1a(bytes: &[u8], mut hash: u64) -> u64 {
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;

fn hsl(hue_degrees: f32, saturation: f32, lightness: f32) -> [f32; 3] {
    let c = (1.0 - (2.0 * lightness - 1.0).abs()) * saturation;
    let h = hue_degrees.rem_euclid(360.0) / 60.0;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = lightness - c / 2.0;
    [(r + m) * 255.0, (g + m) * 255.0, (b + m) * 255.0]
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

const WHITE: [f32; 3] = [255.0, 255.0, 255.0];
const BLACK: [f32; 3] = [0.0, 0.0, 0.0];

/// The requested size scaled down (keeping its aspect) to fit 3840 × 2160, or 2160 × 3840 for portrait.
fn output_size(width: u32, height: u32) -> (u32, u32) {
    let (max_w, max_h) = if width >= height { (MAX_WIDTH, MAX_HEIGHT) } else { (MAX_HEIGHT, MAX_WIDTH) };
    let scale = (f64::from(max_w) / f64::from(width)).min(f64::from(max_h) / f64::from(height)).min(1.0);
    let scaled = |side: u32| ((f64::from(side) * scale).round() as u32).max(1);
    (scaled(width).min(max_w), scaled(height).min(max_h))
}

/// Top, bottom and glow colours: from the prompt's colour words, else from its hash.
fn scheme(prompt: &str, hash: u64) -> ([f32; 3], [f32; 3], [f32; 3]) {
    let named = palette_colours(prompt);
    match named.as_slice() {
        [] => {
            let hue = (hash % 360) as f32;
            (hsl(hue, 0.38, 0.46), hsl(hue + 35.0, 0.42, 0.2), hsl(hue - 25.0, 0.55, 0.74))
        }
        [one] => (*one, mix(*one, BLACK, 0.55), mix(*one, WHITE, 0.5)),
        [first, second] => (*first, mix(*second, BLACK, 0.25), mix(*first, WHITE, 0.5)),
        [first, second, third, ..] => (*first, mix(*second, BLACK, 0.25), *third),
    }
}

fn paint(prompt: &str, width: u32, height: u32, seed: Option<u64>) -> Result<Vec<u8>> {
    use image::ImageEncoder;

    let (w, h) = output_size(width, height);
    let hash = fnv1a(prompt.as_bytes(), FNV_OFFSET);
    let placement = fnv1a(&seed.unwrap_or(0).to_le_bytes(), hash);
    let (top, bottom, glow) = scheme(prompt, hash);

    // Glow centre in the upper half, off-centre by the prompt and seed; radius from the longer side.
    let unit = |bits: u64| (bits & 0xffff) as f32 / 65_535.0;
    let cx = (0.25 + 0.5 * unit(placement)) * w as f32;
    let cy = (0.2 + 0.25 * unit(placement >> 16)) * h as f32;
    let radius = 0.6 * w.max(h) as f32;
    let strength = 0.55;

    let mut pixels = Vec::with_capacity(w as usize * h as usize * 3);
    for y in 0..h {
        let t = if h > 1 { y as f32 / (h - 1) as f32 } else { 0.0 };
        let eased = t * t * (3.0 - 2.0 * t);
        let base = mix(top, bottom, eased);
        let dy = y as f32 - cy;
        for x in 0..w {
            let dx = x as f32 - cx;
            let falloff = (1.0 - (dx * dx + dy * dy).sqrt() / radius).max(0.0);
            let colour = mix(base, glow, falloff * falloff * strength);
            pixels.extend(colour.map(|channel| channel.round().clamp(0.0, 255.0) as u8));
        }
    }
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(&pixels, w, h, image::ExtendedColorType::Rgb8)
        .map_err(|error| AutoPaperError::Internal { detail: format!("encoding the demo image failed: {error}") })?;
    Ok(png)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ImageQuality;

    fn inputs(musts: &[&str], maybes: &[&str], avoids: &[&str], surprise: f32) -> ComposeInputs {
        let strings = |list: &[&str]| list.iter().map(|s| s.to_string()).collect();
        ComposeInputs { musts: strings(musts), maybes: strings(maybes), avoids: strings(avoids), surprise, candidates: 4, echo_of: None }
    }

    fn request(inputs: ComposeInputs) -> ComposeRequest {
        ComposeRequest {
            model: String::new(),
            // Never read: the Demo uses `inputs` only.
            system: "Must: something else entirely".into(),
            user: "Avoid: nothing".into(),
            schema: serde_json::json!({}),
            temperature: 0.7,
            inputs,
        }
    }

    async fn concepts(demo: &Demo, inputs: ComposeInputs) -> Vec<(Concept, Option<String>)> {
        let response = demo.compose(request(inputs)).await.unwrap();
        assert_eq!(response.model, "demo");
        assert_eq!(response.usage, Usage::default());
        response.output["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .map(|candidate| {
                let note = candidate.get("echo_note").and_then(|n| n.as_str()).map(String::from);
                (serde_json::from_value(candidate.clone()).unwrap(), note)
            })
            .collect()
    }

    fn all_text(concept: &Concept) -> Vec<String> {
        let mut fields = vec![
            concept.title.clone(),
            concept.summary.clone(),
            concept.setting.clone(),
            concept.subject.clone(),
            concept.time_of_day.clone(),
            concept.weather.clone(),
            concept.season.clone(),
            concept.style.clone(),
            concept.composition.clone(),
            concept.prompt.clone(),
        ];
        for list in [&concept.elements, &concept.mood, &concept.palette, &concept.keywords_used, &concept.wildcards] {
            fields.extend(list.iter().cloned());
        }
        fields
    }

    #[test]
    fn uses_the_composers_wildcard_rule() {
        assert_eq!(wildcard_limit(0.0), 0);
        assert_eq!(wildcard_limit(0.249), 0);
        assert_eq!(wildcard_limit(0.25), 1);
        assert_eq!(wildcard_limit(0.49), 1);
        assert_eq!(wildcard_limit(0.5), 2);
        assert_eq!(wildcard_limit(0.74), 2);
        assert_eq!(wildcard_limit(0.75), 3);
        assert_eq!(wildcard_limit(1.0), 3);
    }

    #[test]
    fn chooses_a_or_an_by_sound() {
        for (phrase, article) in [
            ("atmospheric photograph", "an"),
            ("oil painting", "an"),
            ("Overgrown carousel", "an"),
            ("hour of rain", "an"),
            ("soft matte painting", "a"),
            ("watercolour illustration", "a"),
            ("unique view", "a"),
            ("European harbour", "a"),
            ("one-off scene", "a"),
            ("", "a"),
        ] {
            assert_eq!(indefinite(phrase), article, "{phrase:?}");
        }
    }

    #[test]
    fn names_each_thing_once() {
        let parts = |list: &[&str]| list.iter().map(|part| part.to_string()).collect::<Vec<_>>();
        assert_eq!(
            distinct_parts(parts(&["a lighthouse", "lighthouse", "lichen-covered rocks", "rocks"]), &[]),
            ["a lighthouse", "lichen-covered rocks"]
        );
        assert_eq!(distinct_parts(parts(&["rocks", "moss", "lichen-covered rocks"]), &[]), ["lichen-covered rocks", "moss"]);
        assert_eq!(
            distinct_parts(parts(&["a stone bridge", "canyon", "rain", "low mist", ""]), &["a desert canyon", "light rain"]),
            ["a stone bridge", "low mist"]
        );
    }

    /// Each word of `text`, lower-cased, with how often it appears.
    fn word_counts(text: &str) -> std::collections::HashMap<String, usize> {
        let mut counts = std::collections::HashMap::new();
        for word in text::words(text) {
            *counts.entry(word).or_insert(0) += 1;
        }
        counts
    }

    #[tokio::test]
    async fn summaries_read_as_english() {
        let keywords = inputs(&["lighthouse", "canyon", "rain"], &["rocks", "stone bridge"], &[], 0.6);
        let mut styles = std::collections::HashSet::new();
        for seed in 0..60 {
            for (concept, _) in concepts(&Demo::new(seed), keywords.clone()).await {
                styles.insert(concept.style.clone());
                for text in [&concept.summary, &concept.prompt] {
                    let words = text::words(text);
                    for pair in words.windows(2) {
                        if pair[0] == "a" {
                            assert_eq!(indefinite(&pair[1]), "a", "seed {seed}: \"a {}\" in {text}", pair[1]);
                        }
                        if pair[0] == "an" {
                            assert_eq!(indefinite(&pair[1]), "an", "seed {seed}: \"an {}\" in {text}", pair[1]);
                        }
                    }
                }
                let counts = word_counts(&concept.summary);
                for keyword in ["lighthouse", "canyon", "bridge"] {
                    assert!(counts.get(keyword).copied().unwrap_or(0) <= 1, "seed {seed}: {keyword} repeated in {}", concept.summary);
                }
            }
        }
        assert!(styles.contains("atmospheric photograph") && styles.contains("oil painting"), "the vowel styles were tried");
    }

    #[tokio::test]
    async fn is_deterministic_for_a_seed() {
        let keywords = inputs(&["rain", "ruins"], &["blue hour", "lanterns", "moss"], &["people"], 0.5);
        let a = Demo::new(42).compose(request(keywords.clone())).await.unwrap().output;
        let b = Demo::new(42).compose(request(keywords.clone())).await.unwrap().output;
        let c = Demo::new(43).compose(request(keywords.clone())).await.unwrap().output;
        assert_eq!(a, b);
        assert_ne!(a, c);

        // Successive calls move on (a fresh idea each time) and replay the same way.
        let demo = Demo::new(42);
        let first = demo.compose(request(keywords.clone())).await.unwrap().output;
        let second = demo.compose(request(keywords.clone())).await.unwrap().output;
        assert_eq!(first, a);
        assert_ne!(first, second);
    }

    #[tokio::test]
    async fn writes_the_requested_number_of_complete_candidates() {
        let demo = Demo::new(1);
        let mut keywords = inputs(&["lighthouse"], &[], &[], 0.3);
        keywords.candidates = 3;
        let written = concepts(&demo, keywords.clone()).await;
        assert_eq!(written.len(), 3);
        for (concept, note) in &written {
            assert!(note.is_none());
            for (name, value) in [("title", &concept.title), ("summary", &concept.summary), ("prompt", &concept.prompt)] {
                assert!(!value.trim().is_empty(), "{name} is empty");
            }
            assert!(concept.title.chars().count() <= TITLE_MAX_CHARS, "{}", concept.title);
            assert!(!concept.setting.is_empty() && !concept.palette.is_empty() && !concept.mood.is_empty());
        }
        keywords.candidates = 0;
        assert_eq!(concepts(&demo, keywords).await.len(), crate::composer::CANDIDATES);
        assert_eq!(concepts(&demo, inputs(&[], &[], &[], 0.0)).await.len(), 4, "no keywords still makes concepts");
    }

    #[tokio::test]
    async fn every_must_is_in_the_prompt_and_keywords_used() {
        for seed in 0..40 {
            let musts = ["rain", "ancient ruins", "blue hour"];
            for (concept, _) in concepts(&Demo::new(seed), inputs(&musts, &["moss"], &["fog"], 0.6)).await {
                for must in musts {
                    assert!(text::mentions(&concept.prompt, must), "seed {seed}: {must} missing from {}", concept.prompt);
                    assert!(concept.keywords_used.iter().any(|k| k == must), "seed {seed}: {must} not in keywords_used");
                }
            }
        }
    }

    #[tokio::test]
    async fn uses_some_maybes_and_varies_them() {
        let maybes = ["lanterns", "moss", "a red door", "cranes"];
        let mut seen = std::collections::HashSet::new();
        for seed in 0..20 {
            for (concept, _) in concepts(&Demo::new(seed), inputs(&[], &maybes, &[], 0.4)).await {
                let used: Vec<&String> = concept.keywords_used.iter().filter(|k| maybes.contains(&k.as_str())).collect();
                assert!(!used.is_empty() && used.len() <= 2, "seed {seed}: {used:?}");
                for maybe in &used {
                    assert!(text::mentions(&concept.prompt, maybe), "{maybe} not in {}", concept.prompt);
                }
                seen.insert(used.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("+"));
            }
        }
        assert!(seen.len() > 3, "Maybes vary across candidates: {seen:?}");
    }

    #[tokio::test]
    async fn never_mentions_an_avoid() {
        // Avoids that hit the vocabularies, the colour words, the prompt template and the Maybes.
        let avoids = ["fog", "mountain", "gold", "night", "lake", "painting", "palette", "wallpaper", "mood", "forest", "lanterns"];
        for seed in 0..60 {
            let echo = (seed % 3 == 0).then(|| Concept {
                title: "Golden lake at night".into(),
                setting: "a quiet alpine lake".into(),
                elements: vec!["fog banks".into(), "a stone pier".into()],
                keywords_used: vec!["lanterns".into(), "pier".into()],
                ..Concept::default()
            });
            let mut keywords = inputs(&["rain", "ruins"], &["lanterns", "moss", "golden light"], &avoids, (seed % 5) as f32 / 4.0);
            keywords.echo_of = echo;
            for (concept, note) in concepts(&Demo::new(seed), keywords).await {
                for text in all_text(&concept).iter().chain(note.as_ref()) {
                    for avoid in avoids {
                        assert!(!text::mentions(text, avoid), "seed {seed}: “{text}” mentions {avoid}");
                    }
                }
            }
        }
    }

    #[tokio::test]
    async fn wildcards_stay_within_surprises_limit() {
        for (surprise, limit) in [(0.0, 0), (0.2, 0), (0.3, 1), (0.6, 2), (0.9, 3), (1.0, 3)] {
            let mut most = 0;
            for seed in 0..15 {
                for (concept, _) in concepts(&Demo::new(seed), inputs(&["rain"], &["moss"], &[], surprise)).await {
                    assert!(concept.wildcards.len() <= limit, "σ {surprise}: {:?}", concept.wildcards);
                    for wildcard in &concept.wildcards {
                        assert!(text::mentions(&concept.prompt, wildcard));
                    }
                    most = most.max(concept.wildcards.len());
                }
            }
            assert_eq!(most, limit, "σ {surprise} reaches its limit sometimes");
        }
    }

    #[tokio::test]
    async fn keywords_steer_time_and_weather() {
        for (concept, _) in concepts(&Demo::new(5), inputs(&["rain", "night"], &[], &[], 0.0)).await {
            assert_eq!(concept.weather, "light rain");
            assert_eq!(concept.time_of_day, "night");
        }
    }

    #[tokio::test]
    async fn echoes_keep_the_original_setting_and_add_a_note() {
        let original = Concept {
            title: "Black ocean, silver structures".into(),
            setting: "a black ocean under a pale sky".into(),
            subject: "silver lattice towers".into(),
            time_of_day: "night".into(),
            weather: "still air".into(),
            season: "winter".into(),
            keywords_used: vec!["ocean".into(), "silver".into()],
            ..Concept::default()
        };
        let mut keywords = inputs(&["desert"], &[], &[], 0.5);
        keywords.echo_of = Some(original.clone());
        for (concept, note) in concepts(&Demo::new(9), keywords).await {
            assert_eq!(concept.setting, original.setting);
            assert_eq!(concept.subject, original.subject);
            assert_ne!(concept.time_of_day, "night");
            assert_ne!(concept.weather, "still air");
            assert_ne!(concept.season, "winter");
            assert!(!concept.keywords_used.iter().any(|k| k == "desert"), "Musts don't apply to echoes");
            assert!(!text::mentions(&concept.prompt, "desert"));
            let note = note.expect("echo_note");
            assert!(note.starts_with("Echo of “Black ocean, silver structures”: now "), "{note}");
            assert!(note.contains(&concept.time_of_day), "{note}");
        }

        // An original setting that now mentions an Avoid is replaced, and its title isn't repeated.
        let mut keywords = inputs(&[], &[], &["ocean", "silver"], 0.5);
        keywords.echo_of = Some(original.clone());
        for (concept, note) in concepts(&Demo::new(9), keywords).await {
            assert!(SETTINGS.contains(&concept.setting.as_str()), "{}", concept.setting);
            assert!(note.unwrap().starts_with("Echo of an earlier image"));
        }
    }

    #[test]
    fn clean_text_falls_back_without_dropping_musts() {
        let avoids = vec!["wallpaper".to_string(), "blue hour".to_string()];
        // "deep blue" + "hour glass" make "blue hour" across the join: optional parts go from the end.
        let parts = vec![
            ("deep blue".to_string(), false),
            ("hour glass".to_string(), true),
            ("rain".to_string(), true),
            ("moss".to_string(), false),
        ];
        let text = clean_text("A desktop wallpaper".into(), &parts, &avoids);
        assert_eq!(text, "Hour glass, rain");
        let simple = [("moss".to_string(), false), ("rain".to_string(), true)];
        assert_eq!(clean_text("A wallpaper of moss and rain".into(), &simple, &avoids), "Moss, rain");
        assert_eq!(clean_text("Fine as is".into(), &parts, &avoids), "Fine as is");
    }

    #[test]
    fn truncates_titles_at_words() {
        assert_eq!(truncate_words("Rain over Coastal Cliffs", 60), "Rain over Coastal Cliffs");
        assert_eq!(truncate_words("Rain over Coastal Cliffs", 12), "Rain over");
        assert_eq!(truncate_words("Supercalifragilistic", 5), "Super");
    }

    fn image_request(prompt: &str, width: u32, height: u32, seed: Option<u64>) -> ImageRequest {
        ImageRequest {
            model: String::new(),
            prompt: prompt.into(),
            width,
            height,
            quality: ImageQuality::High,
            seed,
            progress: None,
            expected_secs: None,
        }
    }

    #[tokio::test]
    async fn paints_a_png_at_the_requested_size_from_palette_words() {
        let demo = Demo::new(0);
        let image =
            demo.generate(image_request("Dusk over salt flats, a palette of deep teal, amber and ivory", 320, 180, Some(3))).await.unwrap();
        assert_eq!(image.mime, "image/png");
        assert_eq!(image.model, "demo");
        assert_eq!(image.reported_cost_microusd, None);
        let decoded = image::load_from_memory_with_format(&image.bytes, image::ImageFormat::Png).unwrap().to_rgb8();
        assert_eq!(decoded.dimensions(), (320, 180));

        // Top row near teal, bottom row near (darkened) amber; the glow lifts the upper half.
        let [r, g, b] = decoded.get_pixel(319, 0).0;
        assert!(i32::from(g) > i32::from(r) + 30 && b > r, "top is teal-ish: {r},{g},{b}");
        let [r, g, b] = decoded.get_pixel(0, 179).0;
        assert!(r > g && g > b, "bottom is amber-ish: {r},{g},{b}");
        let brightness = |p: &image::Rgb<u8>| p.0.iter().map(|c| u32::from(*c)).sum::<u32>();
        let glow = (0..320).map(|x| brightness(decoded.get_pixel(x, 60))).max().unwrap();
        assert!(glow > brightness(decoded.get_pixel(0, 0)), "a gentle glow");
    }

    #[tokio::test]
    async fn images_depend_only_on_the_request() {
        let a = Demo::new(1).generate(image_request("a misty valley at dawn", 64, 36, Some(7))).await.unwrap();
        let b = Demo::new(2).generate(image_request("a misty valley at dawn", 64, 36, Some(7))).await.unwrap();
        let other_seed = Demo::new(1).generate(image_request("a misty valley at dawn", 64, 36, Some(8))).await.unwrap();
        let other_prompt = Demo::new(1).generate(image_request("a dry canyon at noon", 64, 36, Some(7))).await.unwrap();
        assert_eq!(a.bytes, b.bytes);
        assert_ne!(a.bytes, other_seed.bytes);
        assert_ne!(a.bytes, other_prompt.bytes);
    }

    #[test]
    fn without_colour_words_the_colours_come_from_a_stable_hash() {
        assert!(palette_colours("a misty valley at dawn").is_empty());
        let hash = fnv1a("a misty valley at dawn".as_bytes(), FNV_OFFSET);
        assert_eq!(hash, fnv1a("a misty valley at dawn".as_bytes(), FNV_OFFSET));
        let (top, bottom, glow) = scheme("a misty valley at dawn", hash);
        assert_ne!(top, bottom);
        assert!(glow.iter().sum::<f32>() > top.iter().sum::<f32>());
        assert_eq!(fnv1a(b"", FNV_OFFSET), FNV_OFFSET);
        assert_eq!(fnv1a(b"a", FNV_OFFSET), 0xaf63_dc4c_8601_ec8c, "FNV-1a 64 test vector");
        assert_eq!(palette_colours("Golden fields, slate and SLATE skies, teal water, rose"), {
            let c = |n: &str| colour_named(n).map(|[r, g, b]| [f32::from(r), f32::from(g), f32::from(b)]).unwrap();
            vec![c("gold"), c("slate"), c("teal")]
        });
    }

    #[test]
    fn caps_the_size_at_3840_by_2160() {
        assert_eq!(output_size(1920, 1080), (1920, 1080));
        assert_eq!(output_size(3840, 2160), (3840, 2160));
        assert_eq!(output_size(5120, 2880), (3840, 2160));
        assert_eq!(output_size(6016, 3384), (3840, 2160));
        assert_eq!(output_size(3840, 2400), (3456, 2160));
        assert_eq!(output_size(5120, 1440), (3840, 1080));
        assert_eq!(output_size(2160, 3840), (2160, 3840));
        assert_eq!(output_size(3000, 6000), (1920, 3840));
        assert_eq!(output_size(1, 1), (1, 1));
    }

    #[tokio::test]
    async fn rejects_an_empty_size() {
        let result = Demo::new(0).generate(image_request("x", 0, 100, None)).await;
        assert!(matches!(result, Err(AutoPaperError::InvalidInput { .. })), "{result:?}");
    }

    #[tokio::test(start_paused = true)]
    async fn slow_mode_takes_its_delay_and_reports_its_steps() {
        let delay = Duration::from_secs(40);
        let demo = Demo::new(1).with_delay(delay);
        assert_eq!(ImageProvider::steps(&demo, ""), Some(DEMO_STEPS));
        assert_eq!(ImageProvider::steps(&Demo::new(1), ""), None, "fast by default");
        let started = tokio::time::Instant::now();
        demo.compose(request(inputs(&["rain"], &[], &[], 0.3))).await.unwrap();
        assert_eq!(started.elapsed(), Duration::from_secs(10), "writing: a quarter");
        let steps = std::sync::Arc::new(Mutex::new(Vec::new()));
        let sink = steps.clone();
        let progress = super::super::PaintProgress(std::sync::Arc::new(move |step: PaintStep| {
            sink.lock().unwrap().push((tokio::time::Instant::now(), step.done, step.total));
        }));
        let painting = tokio::time::Instant::now();
        let request = ImageRequest { progress: Some(progress), ..image_request("blue", 64, 36, Some(1)) };
        demo.generate(request).await.unwrap();
        assert_eq!(painting.elapsed(), Duration::from_secs(30), "painting: the rest");
        let steps = steps.lock().unwrap().clone();
        let counts: Vec<(u32, u32)> = steps.iter().map(|(_, done, total)| (*done, *total)).collect();
        assert_eq!(counts, (1..=DEMO_STEPS).map(|done| (done, DEMO_STEPS)).collect::<Vec<_>>());
        assert_eq!(steps[0].0 - painting, Duration::from_millis(3750), "evenly spaced");
        assert_eq!(Demo::new(1).with_delay(Duration::from_secs(86_400)).delay, MAX_DELAY);
    }

    #[test]
    fn offers_free_sizes_up_to_4k() {
        let caps = Demo::new(0).capabilities("demo");
        assert!(caps.sizes.is_empty());
        let free = caps.free_size.unwrap();
        assert_eq!((free.step, free.max_side, free.max_pixels), (8, 3840, 8_294_400));
    }
}
