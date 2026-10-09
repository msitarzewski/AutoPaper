//! The composer: keywords + Surprise + taste + memory → instructions for a text model → candidate
//! concepts → deterministic checks → a score. The retry loop (ask, embed, assess novelty, maybe ask
//! again) lives in the engine; everything here is pure and testable.
//!
//! The instructions encode the wallpaper rules (`memory-bank/systemPatterns.md`, "Concept"): landscape
//! desktop wallpaper; no text, letters, logos, watermarks or UI; calm areas where icons sit; every Must;
//! some Maybes (varied across candidates); never anything from Avoid — not even negated, because image
//! models misread negation; wildcards limited by Surprise; recent ideas named so they aren't repeated;
//! `title`/`summary`/`echo_note` written in the person's locale; `summary` written as an image
//! description someone who can't see it would find useful.
//!
//! The system instructions depend only on the Avoid keywords (which filter the example text) and on
//! whether the request is an echo, so they stay byte-identical between calls and providers can cache
//! them; everything that changes per call is in the user instructions.

use serde_json::{Map, Value, json};

use crate::echo::EchoAxis;
use crate::error::{AutoPaperError, Result};
use crate::model::{Appearance, Concept, Keyword, KeywordWeight, SurpriseBand};
use crate::providers::{ComposeInputs, ComposeRequest};
use crate::text;

/// Candidates asked for per call.
pub const CANDIDATES: usize = 4;
/// Longest prompt accepted from the model (characters).
pub const MAX_PROMPT_CHARS: usize = 2500;
/// Longest title (characters; titles are shown in menus). `parse` shortens longer ones.
pub const MAX_TITLE_CHARS: usize = 60;
/// Most items `parse` keeps from any array, the candidates included.
pub const MAX_ITEMS: usize = 12;
/// Most recent summaries named in the instructions.
pub const MAX_RECENT: usize = 20;
/// Most "too close" summaries named on a retry.
const MAX_TOO_CLOSE: usize = 8;
const MAX_CORRECTIONS: usize = 6;
/// Most liked and most disliked taste hints named (each).
const MAX_HINTS: usize = 6;
/// Longest remembered summary quoted back to the model (characters).
const MAX_QUOTED_CHARS: usize = 400;

/// Everything the composer needs for one request.
#[derive(Debug, Clone, Default)]
pub struct ComposeContext {
    pub keywords: Vec<Keyword>,
    /// 0–1.
    pub surprise: f32,
    /// BCP 47.
    pub locale: String,
    /// From `Taste::hints`.
    pub liked: Vec<String>,
    pub disliked: Vec<String>,
    /// Recent summaries, newest first ("don't repeat these"); the first `MAX_RECENT` are used.
    pub recent: Vec<String>,
    /// On a retry: summaries of the remembered wallpapers the last candidates were too close to.
    pub too_close: Vec<String>,
    /// On a retry: what made the last candidates unusable (`correction`), so the model can put it right
    /// instead of making the same mistake again.
    pub corrections: Vec<String>,
    pub echo: Option<EchoBrief>,
    /// The provider declined the last attempt (content policy): ask for gentler, plainly described scenes.
    pub gentler: bool,
    /// The computer's appearance when the person asked for wallpapers to suit it (`Settings::match_system_theme`).
    pub appearance: Option<crate::model::Appearance>,
}

#[derive(Debug, Clone)]
pub struct EchoBrief {
    pub original: Concept,
    /// From `echo::age_text`, e.g. "2 years ago".
    pub age: String,
    pub axes: Vec<EchoAxis>,
}

/// One parsed candidate.
#[derive(Debug, Clone, PartialEq)]
pub struct Composed {
    pub concept: Concept,
    /// Echo requests only, when the model wrote one.
    pub echo_note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    MissingMust(String),
    MentionsAvoid(String),
    EmptyField(&'static str),
    PromptTooLong,
    /// The prompt names the computer itself ("desktop wallpaper", "screenshot", …): some image models then paint a
    /// desktop with a menu bar and dock instead of the scene (Krea 2 Turbo did, twice; systemPatterns "Prompt lessons").
    MentionsScreen(String),
}

/// Words that make image models paint a computer instead of a scene. Skipped when one of the person's own Must or
/// Maybe keywords mentions it (someone may really want "a retro desktop computer").
const SCREEN_WORDS: [&str; 9] = [
    "wallpaper", "desktop", "screenshot", "screensaver", "lock screen", "home screen", "taskbar", "menu bar",
    "user interface",
];

// ── Schema ──────────────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FieldKind {
    Text,
    List,
}

/// The candidate's fields in generation order: OpenAI and Gemini emit keys in the schema's property
/// order, so the model plans the scene first, commits to the keywords it uses, writes the prompt from
/// that plan, and only then names and describes the image it wrote. (The serialized schema keeps this
/// order only when serde_json's `preserve_order` feature is on; otherwise properties are alphabetical,
/// which is still valid, just a weaker plan-then-write sequence. `required` always keeps it.)
const FIELDS: [(&str, FieldKind, &str); 15] = [
    ("setting", FieldKind::Text, "The kind of place, a short English phrase."),
    ("subject", FieldKind::Text, "The focal point, a short English phrase."),
    ("elements", FieldKind::List, "3 to 8 concrete visible things, short English phrases."),
    ("time_of_day", FieldKind::Text, "A short English phrase."),
    ("weather", FieldKind::Text, "A short English phrase."),
    ("season", FieldKind::Text, "A short English phrase."),
    ("mood", FieldKind::List, "2 to 4 English adjectives."),
    ("palette", FieldKind::List, "3 to 6 specific named colours, in English."),
    ("style", FieldKind::Text, "The medium and style, in English."),
    ("composition", FieldKind::Text, "Where the focal point sits and where the calm, low-detail space is, in English."),
    ("keywords_used", FieldKind::List, "The person's keywords this candidate uses, copied exactly as given."),
    ("wildcards", FieldKind::List, "Things added that no keyword asked for; empty when none."),
    (
        "prompt",
        FieldKind::Text,
        "The image prompt: English, one paragraph of 60 to 120 words, describing only what is in the image.",
    ),
    ("title", FieldKind::Text, "A short name of at most 60 characters, in the person's language."),
    (
        "summary",
        FieldKind::Text,
        "One or two sentences in the person's language describing the image for someone who can't see it.",
    ),
];

const ECHO_NOTE: (&str, &str) =
    ("echo_note", "One line in the person's language: Echo of “original title” (when): what changed.");

/// JSON Schema for `{"candidates": [ … ]}`, strict-mode compatible (OpenAI structured outputs and
/// Gemini responseSchema): every property required, `additionalProperties: false`, strings and arrays of
/// strings only. With `echo`, each candidate also has `echo_note`. Besides structure it uses only
/// `description`, `minItems` and `maxItems`, which both providers accept.
pub fn schema(echo: bool) -> Value {
    let mut properties = Map::new();
    let mut required = Vec::new();
    let mut add = |name: &str, kind: FieldKind, description: &str| {
        let property = match kind {
            FieldKind::Text => json!({ "type": "string", "description": description }),
            FieldKind::List => json!({ "type": "array", "items": { "type": "string" }, "description": description }),
        };
        properties.insert(name.to_string(), property);
        required.push(Value::String(name.to_string()));
    };
    for (name, kind, description) in FIELDS {
        add(name, kind, description);
    }
    if echo {
        add(ECHO_NOTE.0, FieldKind::Text, ECHO_NOTE.1);
    }
    json!({
        "type": "object",
        "properties": {
            "candidates": {
                "type": "array",
                "minItems": 1,
                "maxItems": CANDIDATES,
                "items": {
                    "type": "object",
                    "properties": properties,
                    "required": required,
                    "additionalProperties": false
                }
            }
        },
        "required": ["candidates"],
        "additionalProperties": false
    })
}

// ── Numbers ─────────────────────────────────────────────────────────────────────────────────────

/// Surprise clamped to 0–1 (NaN → 0).
fn unit(surprise: f32) -> f32 {
    if surprise.is_nan() { 0.0 } else { surprise.clamp(0.0, 1.0) }
}

/// 0.4 + 0.8 × surprise (callers drop it for models that don't take a temperature).
pub fn temperature(surprise: f32) -> f32 {
    0.4 + 0.8 * unit(surprise)
}

/// The band a Surprise value falls in: Faithful below 0.25, Fresh below 0.5, Adventurous below 0.75, Wild from
/// there (clamped to 0–1; NaN is Faithful). Hosts say the band under the Surprise slider and in its spoken value
/// ("35 percent, fresh"), so they never copy the thresholds.
#[uniffi::export]
pub fn surprise_band(surprise: f32) -> SurpriseBand {
    match unit(surprise) {
        s if s < 0.25 => SurpriseBand::Faithful,
        s if s < 0.5 => SurpriseBand::Fresh,
        s if s < 0.75 => SurpriseBand::Adventurous,
        _ => SurpriseBand::Wild,
    }
}

/// Wildcards (unrequested elements) allowed, by band: 0 (Faithful), ≤1 (Fresh), ≤2 (Adventurous), ≤3 (Wild).
pub fn wildcard_limit(surprise: f32) -> usize {
    match surprise_band(surprise) {
        SurpriseBand::Faithful => 0,
        SurpriseBand::Fresh => 1,
        SurpriseBand::Adventurous => 2,
        SurpriseBand::Wild => 3,
    }
}

/// Selection score: novelty × (1 − 0.3s) + taste × (1 − 0.6s) + jitter × 0.3s, where `jitter` ∈ [0, 1)
/// comes from the caller's RNG.
pub fn score(novelty: f32, taste: f32, surprise: f32, jitter: f32) -> f32 {
    let s = unit(surprise);
    novelty * (1.0 - 0.3 * s) + taste * (1.0 - 0.6 * s) + jitter * 0.3 * s
}

// ── Instructions ────────────────────────────────────────────────────────────────────────────────

/// Builds the system and user instructions, the schema and temperature for `CANDIDATES` candidates.
///
/// Echo requests leave out Must and Maybe keywords (in the instructions and in `inputs`): an echo
/// reinterprets the original, so only Avoid applies. Maybe keywords and taste hints that mention an
/// Avoid keyword are dropped, and example text written here never mentions one.
pub fn build_request(context: &ComposeContext, model: &str) -> ComposeRequest {
    let brief = Brief::new(context);
    let echo = context.echo.is_some();
    ComposeRequest {
        model: model.to_string(),
        system: system_instructions(&brief.avoids, echo),
        user: user_instructions(context, &brief),
        schema: schema(echo),
        temperature: temperature(context.surprise),
        inputs: ComposeInputs {
            musts: brief.musts.clone(),
            maybes: brief.maybes.clone(),
            avoids: brief.avoids.clone(),
            surprise: brief.surprise,
            candidates: CANDIDATES,
            echo_of: context.echo.as_ref().map(|echo| echo.original.clone()),
        },
        expected_secs: None,
    }
}

/// The context's keywords and hints, grouped, ordered and filtered.
struct Brief {
    musts: Vec<String>,
    maybes: Vec<String>,
    avoids: Vec<String>,
    liked: Vec<String>,
    disliked: Vec<String>,
    surprise: f32,
}

impl Brief {
    fn new(context: &ComposeContext) -> Self {
        let mut keywords: Vec<&Keyword> = context.keywords.iter().filter(|k| !k.text.trim().is_empty()).collect();
        keywords.sort_by_key(|k| k.position);
        let texts = |weight: KeywordWeight| -> Vec<String> {
            keywords.iter().filter(|k| k.weight == weight).map(|k| k.text.trim().to_string()).collect()
        };
        let avoids = texts(KeywordWeight::Avoid);
        let echo = context.echo.is_some();
        let (musts, maybes) = if echo {
            (Vec::new(), Vec::new())
        } else {
            let maybes: Vec<String> =
                texts(KeywordWeight::Maybe).into_iter().filter(|m| !mentions_any(m, &avoids)).collect();
            (texts(KeywordWeight::Must), maybes)
        };
        let hints = |list: &[String]| -> Vec<String> {
            list.iter()
                .map(|h| one_line(h, MAX_QUOTED_CHARS))
                .filter(|h| !h.is_empty() && !mentions_any(h, &avoids))
                .take(MAX_HINTS)
                .collect()
        };
        Self {
            liked: hints(&context.liked),
            disliked: hints(&context.disliked),
            musts,
            maybes,
            avoids,
            surprise: unit(context.surprise),
        }
    }
}

fn mentions_any(haystack: &str, terms: &[String]) -> bool {
    terms.iter().any(|term| text::mentions(haystack, term))
}

/// The items of `examples` that mention none of `avoids`, so example text never puts an Avoid keyword
/// in front of the model.
fn without_avoided<'a>(examples: &[&'a str], avoids: &[String]) -> Vec<&'a str> {
    examples.iter().copied().filter(|example| !mentions_any(example, avoids)).collect()
}

/// Example prompts showing the form: medium first, concrete nouns, light, named colours, composition
/// ending on where the calm space is; 60–90 words; positive phrasing only. Two are shown, skipping any
/// that mention an Avoid keyword.
const EXAMPLE_PROMPTS: [&str; 5] = [
    "Wide panoramic photograph of a lone lighthouse on a low basalt headland at blue hour, a calm sea \
     stretching to a soft horizon, faint mist over the water, the lamp's warm beam the only bright accent, \
     palette of deep slate blue, pewter grey and amber, the headland small in the lower right third, a broad \
     smooth gradient sky filling the upper two thirds, long-exposure stillness, gentle contrast, quiet and \
     spacious.",
    "Gouache painting of terraced tea fields folding over rolling hills in late-afternoon spring light, \
     rounded rows of bushes in sage and jade green, a single small farmhouse with a terracotta roof near the \
     left third, pale apricot haze filling the valley beyond, a wide cream-coloured sky with faint brush \
     texture across the top, flat matte shapes, soft edges, warm and calm.",
    "Minimal 3D render of smooth sandstone arches in a quiet desert at dawn, long violet shadows across \
     softly rippled dunes, the low sun just beyond the right edge warming the stone, palette of terracotta, \
     dusty rose and pale lilac, the arches small in the lower third, a vast clean gradient sky filling the \
     upper half, soft global illumination, matte surfaces, crisp yet restful.",
    "Loose watercolour of a birch forest edge in early winter, fresh snow on the gently sloping ground, \
     slender pale trunks spaced rhythmically across the lower half, a silver-white sky with soft washes of \
     pearl and lavender, a single robin on a low branch at the left third as the only warm accent, generous \
     areas of untouched paper, delicate granulation, serene and airy.",
    "Isometric digital illustration of a tiny greenhouse on a small floating island of moss above a soft \
     sea of clouds at golden hour, warm lamplight glowing through the glass panes, palette of moss green, \
     cloud white and honey gold, the island at the right third, a wide open pastel sky stretching across the \
     left two thirds, clean vector shapes, fine grain, whimsical and peaceful.",
];

/// Inflected forms the checker accepts, for the Must rule's example (the first pair with no Avoid).
const INFLECTIONS: [(&str, &str); 4] =
    [("rainy", "rain"), ("stormy", "storm"), ("lanterns", "lantern"), ("waves", "wave")];

/// Kinds of wildcard, scene-setting that isn't one, and media to vary (filtered lists).
const WILDCARD_KINDS: [&str; 4] = ["object", "creature", "structure", "place"];
const SCENE_SETTING: [&str; 5] = ["light", "sky", "ground", "weather", "palette"];
const MEDIA: [&str; 4] = ["photograph", "painting", "illustration", "render"];

/// Calm-area techniques named in the wallpaper rules (a filtered list).
const CALM_AREAS: [&str; 4] = ["open negative space", "soft gradients", "gentle texture", "shallow focus"];

const EXAMPLES_SHOWN: usize = 2;

fn system_instructions(avoids: &[String], echo: bool) -> String {
    let listed = |items: &[&str], last: &str| -> String {
        match without_avoided(items, avoids).as_slice() {
            [] => String::new(),
            [one] => format!(" ({one})"),
            [init @ .., tail] => format!(" ({} {last} {tail})", init.join(", ")),
        }
    };
    let calm = listed(&CALM_AREAS, "and");
    let wildcard_kinds = match without_avoided(&WILDCARD_KINDS, avoids).as_slice() {
        [] => String::new(),
        [one] => format!(": a distinct {one}"),
        [init @ .., tail] => format!(": a distinct {} or {tail}", init.join(", ")),
    };
    let scene_setting = listed(&SCENE_SETTING, "and");
    let media = listed(&MEDIA, "or");
    let inflection =
        INFLECTIONS.iter().find(|(form, base)| !mentions_any(form, avoids) && !mentions_any(base, avoids)).map_or_else(
            || "an inflected form counts".to_string(),
            |(form, base)| format!("an inflected form, such as \"{form}\" for \"{base}\", counts"),
        );
    let n = CANDIDATES;
    let mut out = format!(
        "You are the scene composer inside AutoPaper, an app that makes a new desktop wallpaper for one person \
from their keywords, on a schedule. Your only job is to invent {n} candidate wallpaper scenes and describe each \
one as structured JSON, including a prompt for an image-generation model. The app checks the candidates, keeps \
the best one and sends its prompt to the image model. Nobody reads your answer except the app.

## What makes a good desktop wallpaper
Every candidate must be:
1. A landscape image, wider than tall, that fills the whole frame edge to edge.
2. Free of writing: no text, letters, numbers, signs, logos, watermarks, signatures, user-interface elements, \
frames or borders. Leave out objects that usually have writing on them unless a keyword asks for them.
3. Calm where the icons sit: large areas low in detail{calm}, so desktop icons and open apps stay easy to see \
on top of it. One clear focal point, placed off-centre and not filling the frame.
4. Easy to live with for hours: a coherent palette, gentle contrast and even light; never busy, cluttered, harsh, \
garish or unsettling. Beautiful at first glance and still restful at the end of the day.
5. Suitable for any workplace: no nudity, gore or violence, no real, identifiable individuals, no brands, no \
copyrighted characters, and no named artists.

## The person's keywords
The user message lists the keywords in three groups.
- Must: every candidate includes every Must keyword, clearly visible in the scene. Write each Must keyword's own \
words into the prompt, a multi-word keyword together and in order ({inflection}). If a keyword isn't English, \
keep it in the prompt exactly as written and follow it with its English meaning in parentheses.
- Maybe: optional ingredients. Each candidate uses some of them, or none, and different candidates use different \
ones, so that across the set every Maybe gets a turn where possible.
- Avoid: never part of any candidate. Never write an Avoid keyword, a synonym or translation of it, or anything \
that would show it, in any field, not even negated (\"no …\", \"without …\", \"free of …\"): image models treat \
any mention as a request. Leave it out completely and describe what is there instead. If a Maybe, the original \
of an echo, or an idea of your own would bring in something from Avoid, drop that part.
- Wildcards are things nobody asked for, added for interest{wildcard_kinds}. Scene-setting{scene_setting} \
doesn't count as a wildcard. The user message gives the most wildcards allowed per candidate; list the ones you \
add in \"wildcards\".

## Surprise
Surprise, from 0 to 1, sets how freely you read the keywords. The user message gives its value and meaning:
- below 0.25, faithful: each keyword as anyone would picture it, literally, in a natural, believable scene.
- 0.25 to 0.5, fresh: believable scenes, each with one unexpected choice of viewpoint, light, season, era or \
medium.
- 0.5 to 0.75, adventurous: free interpretations, with unexpected settings, scales, eras and styles.
- 0.75 and above, wild: the keywords are loose inspiration for surprising, dreamlike or surreal scenes in bold \
styles and media. Must keywords are still named in the prompt and visible, in whatever form you choose.

## Make the {n} candidates different
They are alternatives for the app to choose between, so make them genuinely different ideas, not variations of \
one: a different kind of place, subject and focal point, a different time of day or weather, a different \
palette and a different composition in each; vary the medium too{media}. Each must also differ clearly from \
the recent wallpapers the user message lists.

## Fields
Fill every field of every candidate; never leave one empty. Write title, summary and echo_note in the person's \
language, keywords_used exactly as the keywords are written, and every other field in English.
- setting: the kind of place, a short phrase.
- subject: the focal point, a short phrase.
- elements: 3 to 8 concrete visible things, short phrases.
- time_of_day, weather, season: a short phrase each; if one barely applies, give the closest honest description.
- mood: 2 to 4 adjectives.
- palette: 3 to 6 named colours, specific rather than generic.
- style: the medium and style.
- composition: where the focal point sits and where the calm, low-detail space is.
- keywords_used: the person's Must and Maybe keywords this candidate uses, copied exactly as written.
- wildcards: the wildcards you added, or an empty list.
- prompt: the image prompt; see below.
- title: a short, evocative name for the wallpaper, at most 60 characters, without quotation marks, in the \
person's language.
- summary: one or two sentences in the person's language describing the finished image for someone who can't \
see it: what is shown, where things are, the light and the colours. Describe it directly, without starting \
\"An image of\". The app also remembers each wallpaper by its summary, so make it specific.

## Writing the prompt
- Always in English, whatever the person's language: one paragraph of 60 to 120 words.
- Start with the medium and the scene; then the subject and where it sits, the key elements as concrete nouns, \
the time of day and quality of light, the weather and atmosphere, the palette as named colours, and the \
composition, ending with where the wide calm areas are.
- Describe only what is in the image. No negations of any kind (\"no\", \"not\", \"without\", \"free of\"), no \
instructions to the model, no words to be written in the image, no sizes or aspect ratios.
- Never call it a wallpaper, desktop, screen or background in the prompt, and never mention icons, menus or a user \
interface: some image models then show a computer instead of the scene. It is simply a full-bleed scene.
- Include every Must keyword's words."
    );

    let examples: Vec<&str> = without_avoided(&EXAMPLE_PROMPTS, avoids).into_iter().take(EXAMPLES_SHOWN).collect();
    if !examples.is_empty() {
        out.push_str(
            "\n\n## Example prompts\nThese show the form only. Take your content from the keywords, never from these.",
        );
        for example in examples {
            out.push_str("\n- ");
            out.push_str(example);
        }
    }

    if echo {
        out.push_str(
            "\n\n## Echoes
This request is an echo: a new interpretation of a wallpaper the person saw some time ago, returning changed, \
like a memory revisited. The user message describes the original and the changes to make.
- Keep the original's essence: the same kind of place and its key elements, so it is recognisable at once.
- Make every requested change, clearly. Each candidate makes its own, different choices for those changes.
- Must and Maybe keywords don't apply to echoes; Avoid still does. Leave out anything from Avoid even if the \
original had it.
- Surprise sets how far the changes go, rather than how freely to read keywords.
- Write a new prompt from scratch rather than editing the original's.
- keywords_used: the original's keywords that this candidate still uses.
- echo_note: one line in the person's language, in the form: Echo of “original title” (when it was first shown): \
what changed, in a few words.",
        );
    }

    out.push_str(&format!(
        "\n\n## Output\nReturn only the JSON object the schema describes: {{\"candidates\": [...]}} with exactly {n} \
candidates."
    ));
    out
}

/// What Surprise means for reading the keywords, indexed by `wildcard_limit` (which shares Surprise's
/// thresholds: below 0.25, 0.5, 0.75, and above).
const SURPRISE_BANDS: [&str; 4] = [
    "faithful and literal: each keyword as anyone would picture it, in a natural, believable scene",
    "fresh but believable: natural scenes, each with one unexpected choice of viewpoint, light, season, era or medium",
    "adventurous: interpret the keywords freely, with unexpected settings, scales, eras and styles",
    "wild: the keywords are loose inspiration for surprising, dreamlike or surreal scenes in bold styles and media; \
Must keywords are still named in the prompt and visible, in whatever form you choose",
];

/// How far an echo's changes go, by the same bands.
const ECHO_DISTANCE: [&str; 4] = [
    "keep the changes gentle, so the original is recognisable at a glance",
    "make the changes clear while keeping the original's look and feel",
    "make the changes bold; the medium and mood may change too",
    "reimagine it boldly and push every change far, while the place and its key elements stay recognisable",
];

fn user_instructions(context: &ComposeContext, brief: &Brief) -> String {
    let band = wildcard_limit(brief.surprise).min(SURPRISE_BANDS.len() - 1);
    let mut lines: Vec<String> = Vec::new();

    match &context.echo {
        Some(echo) => echo_section(&mut lines, echo, &brief.avoids),
        None => keyword_section(&mut lines, brief),
    }

    lines.push(String::new());
    let scale = format!("Surprise: {:.2} on a scale from 0 (faithful) to 1 (wild)", brief.surprise);
    lines.push(match context.echo {
        Some(_) => format!("{scale}. For this echo it sets how far the changes go: {}.", ECHO_DISTANCE[band]),
        None => format!("{scale}, so {}.", SURPRISE_BANDS[band]),
    });
    lines.push(match band {
        0 => "Wildcards: none. Add nothing that no keyword asks for beyond setting the scene.".to_string(),
        1 => "Wildcards: at most 1 per candidate.".to_string(),
        limit => format!("Wildcards: at most {limit} per candidate."),
    });

    lines.push(String::new());
    let localized = if context.echo.is_some() { "title, summary and echo_note" } else { "title and summary" };
    lines.push(format!(
        "Language: write {localized} in {}. Copy keywords_used exactly as the keywords are written. Write every \
other field, and always the prompt, in English.",
        language_label(&context.locale)
    ));

    if !brief.liked.is_empty() || !brief.disliked.is_empty() {
        lines.push(String::new());
        lines.push(
            "Taste, learned from the person's likes and dislikes. These are soft preferences: let them nudge your \
choices, never override the keywords."
                .to_string(),
        );
        if !brief.liked.is_empty() {
            lines.push(format!("- Tends to like: {}", brief.liked.join("; ")));
        }
        if !brief.disliked.is_empty() {
            lines.push(format!("- Tends to dislike: {}", brief.disliked.join("; ")));
        }
    }

    let recent = quoted(&context.recent, MAX_RECENT);
    if !recent.is_empty() {
        lines.push(String::new());
        lines.push(if context.echo.is_some() {
            "Recent wallpapers, newest first. Apart from the original, don't repeat these:".to_string()
        } else {
            "Recent wallpapers, newest first. Don't repeat these ideas; choose different places, subjects, light and \
palettes:"
                .to_string()
        });
        lines.extend(recent.iter().enumerate().map(|(i, summary)| format!("{}. {summary}", i + 1)));
    }

    let too_close = quoted(&context.too_close, MAX_TOO_CLOSE);
    if !too_close.is_empty() {
        lines.push(String::new());
        lines.push(if context.echo.is_some() {
            "Your previous candidates were too close to these other past wallpapers. Keep the original's essence, but \
take the changes in a clearly different direction from these:"
                .to_string()
        } else {
            "Your previous candidates were too close to these past wallpapers. Go somewhere clearly different: \
another kind of place, another subject, other light and another palette."
                .to_string()
        });
        lines.extend(too_close.iter().map(|summary| format!("- {summary}")));
    }

    let corrections = quoted(&context.corrections, MAX_CORRECTIONS);
    if !corrections.is_empty() {
        lines.push(String::new());
        lines.push("Your previous candidates couldn't be used. Put these right in every candidate:".to_string());
        lines.extend(corrections.iter().map(|correction| format!("- {correction}")));
    }

    if let Some(appearance) = context.appearance {
        lines.push(String::new());
        lines.push(
            match appearance {
                Appearance::Light => {
                    "The person's computer is in light mode. Favour wallpapers that sit well on a light desktop: bright, \
airy, light-toned, with dark icons and text staying legible on top. Only the keywords can override this."
                }
                Appearance::Dark => {
                    "The person's computer is in dark mode. Favour wallpapers that sit well on a dark desktop: deep, \
moody, dark-toned, with light icons and text staying legible on top. Only the keywords can override this."
                }
            }
            .to_string(),
        );
    }

    if context.gentler {
        lines.push(String::new());
        lines.push(
            "The image service declined the previous attempt. Keep every candidate gentle and plainly described: \
nothing violent, gory, frightening or explicit, and no named real persons, brands or artworks."
                .to_string(),
        );
    }

    lines.push(String::new());
    lines.push(if context.echo.is_some() {
        "Before answering, check each candidate: it keeps the original's essence and makes every requested change; \
nothing from Avoid appears in any field; the prompt is English, 60 to 120 words, with no negations; the candidates \
differ from each other."
            .to_string()
    } else {
        "Before answering, check each candidate: every Must keyword's words are in its prompt; nothing from Avoid \
appears in any field; the prompt is English, 60 to 120 words, with no negations; the candidates differ from each \
other and from the recent wallpapers."
            .to_string()
    });
    lines.join("\n")
}

fn keyword_section(lines: &mut Vec<String>, brief: &Brief) {
    lines.push(format!("Compose {CANDIDATES} wallpaper candidates."));
    if brief.musts.is_empty() && brief.maybes.is_empty() {
        lines.push(
            "The person has no Must or Maybe keywords right now: choose varied scenes with broad appeal.".to_string(),
        );
    }
    list_section(lines, "Must (every candidate includes every one of these):", &brief.musts);
    list_section(lines, "Maybe (use some; vary them across candidates):", &brief.maybes);
    list_section(lines, "Avoid (never part of any candidate, never mentioned in any field):", &brief.avoids);
}

fn echo_section(lines: &mut Vec<String>, echo: &EchoBrief, avoids: &[String]) {
    let original = &echo.original;
    let age = one_line(&echo.age, 60);
    lines.push(format!("Compose {CANDIDATES} echo candidates."));
    lines.push(String::new());
    lines.push(if age.is_empty() {
        "The original wallpaper:".to_string()
    } else {
        format!("The original wallpaper, first shown {age}:")
    });
    let fields: [(&str, String); 13] = [
        ("Title", original.title.clone()),
        ("Summary", original.summary.clone()),
        ("Setting", original.setting.clone()),
        ("Subject", original.subject.clone()),
        ("Elements", original.elements.join("; ")),
        ("Time of day", original.time_of_day.clone()),
        ("Weather", original.weather.clone()),
        ("Season", original.season.clone()),
        ("Mood", original.mood.join(", ")),
        ("Palette", original.palette.join(", ")),
        ("Style", original.style.clone()),
        ("Composition", original.composition.clone()),
        ("Made from the keywords", original.keywords_used.join(", ")),
    ];
    for (label, value) in fields {
        let value = one_line(&value, MAX_QUOTED_CHARS);
        if !value.is_empty() {
            lines.push(format!("- {label}: {value}"));
        }
    }
    let prompt = one_line(&original.prompt, MAX_PROMPT_CHARS);
    if !prompt.is_empty() {
        lines.push(format!("- Its image prompt: {prompt}"));
    }

    lines.push(String::new());
    lines.push(
        "Keep its essence (the same kind of place and its key elements) and change it like this; each candidate \
makes all of these changes, in its own way:"
            .to_string(),
    );
    if echo.axes.is_empty() {
        lines.push("- Change whatever you like about its weather, light, season or viewpoint.".to_string());
    }
    lines.extend(echo.axes.iter().map(|&axis| format!("- {}", axis_instruction(axis, avoids))));
    if echo.axes.contains(&EchoAxis::Medium) {
        // Memory compares title, summary and scene fields, not `style`: a new medium only shows there if named.
        lines.push("Name the new medium in the title or summary too.".to_string());
    }

    let title = one_line(&original.title, MAX_TITLE_CHARS * 2);
    let when = if age.is_empty() { "when it was first shown".to_string() } else { age };
    lines.push(format!(
        "echo_note, in the person's language: Echo of “{title}” ({when}): what changed, in a few words."
    ));

    if !avoids.is_empty() {
        list_section(
            lines,
            "Avoid (never part of any candidate, never mentioned in any field, even where the original had it):",
            avoids,
        );
    }
}

/// An echo axis's instruction. When its examples mention an Avoid keyword, the examples are cut (at the
/// first parenthesis or colon), so the request never suggests something the person avoids.
fn axis_instruction(axis: EchoAxis, avoids: &[String]) -> String {
    let full = axis.instruction();
    if !mentions_any(full, avoids) {
        return full.to_string();
    }
    let core = full.split(['(', ':']).next().unwrap_or(full).trim_end();
    format!("{}.", core.trim_end_matches('.'))
}

fn list_section(lines: &mut Vec<String>, heading: &str, items: &[String]) {
    lines.push(String::new());
    if items.is_empty() {
        let label = heading.split(' ').next().unwrap_or(heading);
        lines.push(format!("{label}: none."));
    } else {
        lines.push(heading.to_string());
        lines.extend(items.iter().map(|item| format!("- {item}")));
    }
}

/// Up to `limit` non-empty entries, each on one line and at most `MAX_QUOTED_CHARS` long.
fn quoted(items: &[String], limit: usize) -> Vec<String> {
    items.iter().map(|item| one_line(item, MAX_QUOTED_CHARS)).filter(|item| !item.is_empty()).take(limit).collect()
}

/// Whitespace and control characters collapsed to single spaces, trimmed, at most `max_chars` long
/// (shortened with "…").
fn one_line(text: &str, max_chars: usize) -> String {
    let cleaned: String = text.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    shorten(&cleaned.split_whitespace().collect::<Vec<_>>().join(" "), max_chars)
}

/// `text` if it fits in `max_chars`; otherwise cut at a word boundary (or mid-word when the first word
/// alone is too long) and ended with "…", at most `max_chars` in all.
fn shorten(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let room = max_chars.saturating_sub(1);
    let head: String = text.chars().take(room).collect();
    let next_is_break = text.chars().nth(room).is_none_or(char::is_whitespace);
    let cut = if next_is_break {
        head.as_str()
    } else {
        match head.rfind(char::is_whitespace) {
            Some(space) if space > 0 => &head[..space],
            _ => head.as_str(),
        }
    };
    let cut = cut.trim_end_matches(|c: char| c.is_whitespace() || matches!(c, ',' | ';' | ':' | '-' | '–' | '—'));
    if cut.is_empty() || max_chars == 0 {
        return text.chars().take(max_chars).collect();
    }
    format!("{cut}…")
}

/// "German (de-DE)": the language a BCP 47 (or POSIX, `de_DE.UTF-8`) locale names, so smaller models
/// don't have to decode the tag; unknown tags are passed through as "the language of locale …".
fn language_label(locale: &str) -> String {
    let tag: String = locale
        .split('.')
        .next()
        .unwrap_or_default()
        .trim()
        .replace('_', "-")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .take(35)
        .collect();
    let subtags: Vec<String> = tag.split('-').filter(|s| !s.is_empty()).map(str::to_ascii_lowercase).collect();
    let Some(primary) = subtags.first() else { return "English".to_string() };
    let has = |subtag: &str| subtags.iter().skip(1).any(|s| s == subtag);
    let name = match primary.as_str() {
        "zh" if has("hant") || has("tw") || has("hk") || has("mo") => Some("Traditional Chinese"),
        "zh" => Some("Simplified Chinese"),
        "pt" if has("br") => Some("Brazilian Portuguese"),
        "pt" if has("pt") => Some("European Portuguese"),
        "und" => return "English".to_string(),
        other => LANGUAGES.iter().find(|(code, _)| *code == other).map(|(_, name)| *name),
    };
    match name {
        Some(name) => format!("{name} ({tag})"),
        None => format!("the language of locale {tag}"),
    }
}

const LANGUAGES: [(&str, &str); 44] = [
    ("ar", "Arabic"),
    ("bg", "Bulgarian"),
    ("bn", "Bengali"),
    ("ca", "Catalan"),
    ("cs", "Czech"),
    ("cy", "Welsh"),
    ("da", "Danish"),
    ("de", "German"),
    ("el", "Greek"),
    ("en", "English"),
    ("es", "Spanish"),
    ("et", "Estonian"),
    ("eu", "Basque"),
    ("fa", "Persian"),
    ("fi", "Finnish"),
    ("fr", "French"),
    ("ga", "Irish"),
    ("gl", "Galician"),
    ("he", "Hebrew"),
    ("hi", "Hindi"),
    ("hr", "Croatian"),
    ("hu", "Hungarian"),
    ("id", "Indonesian"),
    ("is", "Icelandic"),
    ("it", "Italian"),
    ("ja", "Japanese"),
    ("ko", "Korean"),
    ("lt", "Lithuanian"),
    ("lv", "Latvian"),
    ("ms", "Malay"),
    ("nb", "Norwegian Bokmål"),
    ("nl", "Dutch"),
    ("nn", "Norwegian Nynorsk"),
    ("no", "Norwegian"),
    ("pl", "Polish"),
    ("pt", "Portuguese"),
    ("ro", "Romanian"),
    ("ru", "Russian"),
    ("sk", "Slovak"),
    ("sl", "Slovenian"),
    ("sv", "Swedish"),
    ("th", "Thai"),
    ("tr", "Turkish"),
    ("uk", "Ukrainian"),
];

// ── Parsing ─────────────────────────────────────────────────────────────────────────────────────

/// Parses a provider's structured output. Tolerates a JSON string containing the object (also inside
/// a Markdown code fence or surrounded by prose), a bare array of candidates, a single candidate
/// object, list fields given as one comma-separated string, and missing optional arrays (→ empty).
/// Trims strings, removes quotation marks around titles and shortens titles to `MAX_TITLE_CHARS`; caps
/// arrays (the candidates too) at `MAX_ITEMS`. A candidate can be read when it is an object with at
/// least one known field. Fails with `InvalidResponse` when no candidate can be read.
pub fn parse(output: &Value, echo: bool) -> Result<Vec<Composed>> {
    let items = locate_candidates(output, 0).unwrap_or_default();
    let composed: Vec<Composed> = items
        .iter()
        .filter_map(|item| match item {
            Value::String(s) => embedded_json(s).and_then(|value| read_candidate(&value, echo)),
            other => read_candidate(other, echo),
        })
        .take(MAX_ITEMS)
        .collect();
    if composed.is_empty() {
        return Err(AutoPaperError::InvalidResponse {
            detail: "the text model's output had no readable candidates".into(),
        });
    }
    Ok(composed)
}

/// The candidate values in `value`: `{"candidates": …}`, a bare array, a single candidate object, or a
/// string holding any of these (followed at most twice, for JSON double-encoded as a string).
fn locate_candidates(value: &Value, depth: u8) -> Option<Vec<Value>> {
    match value {
        Value::Object(map) => match map.get("candidates") {
            Some(inner) => locate_candidates(inner, depth),
            None if is_candidate(map) => Some(vec![value.clone()]),
            None => None,
        },
        Value::Array(items) => Some(items.clone()),
        Value::String(s) if depth < 2 => locate_candidates(&embedded_json(s)?, depth + 1),
        _ => None,
    }
}

fn is_candidate(map: &Map<String, Value>) -> bool {
    FIELDS.iter().any(|(name, _, _)| map.contains_key(*name)) || map.contains_key(ECHO_NOTE.0)
}

/// JSON inside a string: as is, inside a Markdown code fence, or between the first `{`/`[` and the last
/// `}`/`]` when a model wrapped it in prose.
fn embedded_json(s: &str) -> Option<Value> {
    let trimmed = s.trim();
    let unfenced = trimmed
        .strip_prefix("```")
        .map(|rest| {
            let body = rest.split_once('\n').map_or("", |(_, body)| body);
            body.trim_end().strip_suffix("```").unwrap_or(body)
        })
        .unwrap_or(trimmed)
        .trim();
    if let Ok(value) = serde_json::from_str(unfenced) {
        return Some(value);
    }
    let start = trimmed.find(['{', '['])?;
    let end = trimmed.rfind(['}', ']'])?;
    (end > start).then(|| serde_json::from_str(&trimmed[start..=end]).ok()).flatten()
}

fn read_candidate(value: &Value, echo: bool) -> Option<Composed> {
    let map = value.as_object().filter(|map| is_candidate(map))?;
    let text = |key: &str| read_text(map.get(key));
    let list = |key: &str| read_list(map.get(key));
    let concept = Concept {
        title: shorten(strip_quotes(&text("title")), MAX_TITLE_CHARS),
        summary: text("summary"),
        setting: text("setting"),
        subject: text("subject"),
        elements: list("elements"),
        time_of_day: text("time_of_day"),
        weather: text("weather"),
        season: text("season"),
        mood: list("mood"),
        palette: list("palette"),
        style: text("style"),
        composition: text("composition"),
        keywords_used: list("keywords_used"),
        wildcards: list("wildcards"),
        prompt: text("prompt"),
    };
    let echo_note = if echo { Some(text(ECHO_NOTE.0)).filter(|note| !note.is_empty()) } else { None };
    Some(Composed { concept, echo_note })
}

/// A string field: trimmed text, a number's digits, or a list's items joined with ", "; else empty.
fn read_text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(s)) => clean(s),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Array(_)) => read_list(value).join(", "),
        _ => String::new(),
    }
}

/// A list field: its string (or number) items, trimmed, empties dropped, at most `MAX_ITEMS`; a single
/// string is split at commas, semicolons and line breaks.
fn read_list(value: Option<&Value>) -> Vec<String> {
    let items: Vec<String> = match value {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| match item {
                Value::String(s) => Some(clean(s)),
                Value::Number(n) => Some(n.to_string()),
                _ => None,
            })
            .collect(),
        Some(Value::String(s)) => s.split([',', ';', '\n']).map(clean).collect(),
        _ => Vec::new(),
    };
    items.into_iter().filter(|item| !item.is_empty()).take(MAX_ITEMS).collect()
}

/// A model's text with control characters (terminal escapes, line breaks, tabs) as spaces, trimmed: it is
/// stored, shown and logged, and must not be able to drive a terminal.
fn clean(text: &str) -> String {
    let spaced: String = text.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    spaced.trim().to_string()
}

/// Removes matching quotation marks around a whole title (models sometimes quote titles).
fn strip_quotes(title: &str) -> &str {
    const PAIRS: [(char, char); 6] = [('"', '"'), ('\'', '\''), ('“', '”'), ('‘', '’'), ('«', '»'), ('「', '」')];
    for (open, close) in PAIRS {
        if let Some(inner) = title.strip_prefix(open).and_then(|rest| rest.strip_suffix(close)) {
            return inner.trim();
        }
    }
    title
}

// ── Checks ──────────────────────────────────────────────────────────────────────────────────────

/// Deterministic checks (see `text::mentions`): every Must keyword appears in `prompt` (not for
/// echoes), no Avoid keyword appears in `prompt`, `title`, or `summary`; title, summary and prompt are
/// non-empty; prompt ≤ `MAX_PROMPT_CHARS`. Empty = valid. Problems come in that field order: empty
/// fields, prompt length, missing Musts, then Avoid mentions, keywords in position order.
/// What a problem asks of the model on a retry, in the instructions' own words.
pub fn correction(problem: &Problem) -> String {
    match problem {
        Problem::MissingMust(keyword) => {
            format!("The prompt left out the Must keyword \u{201c}{keyword}\u{201d}: write its own words in the prompt.")
        }
        Problem::MentionsAvoid(keyword) => format!(
            "A field mentioned the Avoid keyword \u{201c}{keyword}\u{201d}: leave it out of every field, even negated."
        ),
        Problem::EmptyField(field) => format!("The {field} was empty: write one."),
        Problem::PromptTooLong => "The prompt was too long: keep it to 60 to 120 words.".to_string(),
        Problem::MentionsScreen(word) => {
            format!("The prompt said \u{201c}{word}\u{201d}: describe only the scene, never the computer it is shown on.")
        }
    }
}

pub fn check(concept: &Concept, keywords: &[Keyword], echo: bool) -> Vec<Problem> {
    let mut problems = Vec::new();
    for (name, value) in [("title", &concept.title), ("summary", &concept.summary), ("prompt", &concept.prompt)] {
        if value.trim().is_empty() {
            problems.push(Problem::EmptyField(name));
        }
    }
    if concept.prompt.chars().count() > MAX_PROMPT_CHARS {
        problems.push(Problem::PromptTooLong);
    }

    let mut ordered: Vec<&Keyword> = keywords.iter().filter(|k| !k.text.trim().is_empty()).collect();
    ordered.sort_by_key(|k| k.position);
    if !echo {
        problems.extend(
            ordered
                .iter()
                .filter(|k| k.weight == KeywordWeight::Must && !text::mentions(&concept.prompt, &k.text))
                .map(|k| Problem::MissingMust(k.text.clone())),
        );
    }
    problems.extend(
        ordered
            .iter()
            .filter(|k| k.weight == KeywordWeight::Avoid)
            .filter(|k| {
                [&concept.prompt, &concept.title, &concept.summary].iter().any(|field| text::mentions(field, &k.text))
            })
            .map(|k| Problem::MentionsAvoid(k.text.clone())),
    );
    let wanted = |word: &str| {
        ordered.iter().any(|k| k.weight != KeywordWeight::Avoid && (text::mentions(&k.text, word) || text::mentions(word, &k.text)))
    };
    problems.extend(
        SCREEN_WORDS
            .iter()
            .filter(|word| text::mentions(&concept.prompt, word) && !wanted(word))
            .map(|word| Problem::MentionsScreen((*word).to_string())),
    );
    problems
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompts_that_name_the_computer_are_rejected_unless_asked_for() {
        let mut concept = Concept {
            title: "Lake".into(),
            summary: "A lake.".into(),
            prompt: "Desktop wallpaper, a calm lake at dawn".into(),
            ..Concept::default()
        };
        let problems = check(&concept, &[], false);
        assert!(problems.contains(&Problem::MentionsScreen("wallpaper".into())), "{problems:?}");
        assert!(problems.contains(&Problem::MentionsScreen("desktop".into())), "{problems:?}");

        concept.prompt = "Watercolour of a calm lake at dawn, wide open sky".into();
        assert!(check(&concept, &[], false).is_empty());

        // The person asked for it: a Must keyword mentioning "desktop" lets the word through.
        concept.prompt = "Photograph of a retro desktop computer on a wooden desk by a window".into();
        let wants = [keyword("retro desktop computer", KeywordWeight::Must, 0)];
        assert!(check(&concept, &wants, false).is_empty(), "{:?}", check(&concept, &wants, false));
    }

    #[test]
    fn instructions_forbid_naming_the_computer() {
        let request = build_request(&ComposeContext { surprise: 0.3, locale: "en-US".into(), ..Default::default() }, "");
        assert!(request.system.contains("Never call it a wallpaper, desktop, screen or background in the prompt"));
        for example in EXAMPLE_PROMPTS {
            assert!(SCREEN_WORDS.iter().all(|w| !text::mentions(example, w)), "{example}");
        }
    }

    fn keyword(text: &str, weight: KeywordWeight, position: u32) -> Keyword {
        Keyword { id: format!("k{position}"), text: text.into(), weight, position, created_at: 0 }
    }

    fn keywords(musts: &[&str], maybes: &[&str], avoids: &[&str]) -> Vec<Keyword> {
        let groups = [(musts, KeywordWeight::Must), (maybes, KeywordWeight::Maybe), (avoids, KeywordWeight::Avoid)];
        let mut out = Vec::new();
        for (texts, weight) in groups {
            for text in texts {
                let position = out.len() as u32;
                out.push(keyword(text, weight, position));
            }
        }
        out
    }

    fn context(musts: &[&str], maybes: &[&str], avoids: &[&str], surprise: f32) -> ComposeContext {
        ComposeContext {
            keywords: keywords(musts, maybes, avoids),
            surprise,
            locale: "en-US".into(),
            ..ComposeContext::default()
        }
    }

    fn original() -> Concept {
        Concept {
            title: "Black ocean, silver structures".into(),
            summary: "Tall silver towers rise from a black ocean under a pale moon.".into(),
            setting: "open ocean".into(),
            subject: "silver towers".into(),
            elements: vec!["silver towers".into(), "black water".into(), "pale moon".into()],
            time_of_day: "night".into(),
            weather: "calm".into(),
            season: "autumn".into(),
            mood: vec!["still".into(), "mysterious".into()],
            palette: vec!["black".into(), "silver".into()],
            style: "photograph".into(),
            composition: "towers at the right third, open sky to the left".into(),
            keywords_used: vec!["ocean".into()],
            wildcards: vec![],
            prompt: "Photograph of tall silver towers rising from a black ocean at night.".into(),
        }
    }

    fn echo_context(axes: Vec<EchoAxis>, avoids: &[&str], surprise: f32) -> ComposeContext {
        ComposeContext {
            keywords: keywords(&["lanterns"], &["moss"], avoids),
            surprise,
            locale: "de-DE".into(),
            echo: Some(EchoBrief { original: original(), age: "2 years ago".into(), axes }),
            ..ComposeContext::default()
        }
    }

    fn candidate_json(prompt: &str) -> Value {
        json!({
            "setting": "harbour", "subject": "boats", "elements": ["boats", "pier"], "time_of_day": "dawn",
            "weather": "mist", "season": "spring", "mood": ["calm"], "palette": ["grey", "rose"],
            "style": "photograph", "composition": "boats lower left", "keywords_used": ["boats"],
            "wildcards": [], "prompt": prompt, "title": "Harbour at dawn", "summary": "Boats rest in a misty harbour."
        })
    }

    fn concept(prompt: &str, title: &str, summary: &str) -> Concept {
        Concept { prompt: prompt.into(), title: title.into(), summary: summary.into(), ..Concept::default() }
    }

    // ── schema ──

    /// Asserts strict-mode rules at every object level and returns the keywords used anywhere.
    fn assert_strict(node: &Value, path: &str, used: &mut Vec<String>) {
        let Some(map) = node.as_object() else { panic!("{path}: schema node is not an object") };
        used.extend(map.keys().cloned());
        match map.get("type").and_then(Value::as_str) {
            Some("object") => {
                let properties = map["properties"].as_object().unwrap_or_else(|| panic!("{path}: no properties"));
                let mut names: Vec<&str> = properties.keys().map(String::as_str).collect();
                let mut required: Vec<&str> =
                    map["required"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
                names.sort_unstable();
                required.sort_unstable();
                assert_eq!(names, required, "{path}: required must list every property");
                assert_eq!(map.get("additionalProperties"), Some(&json!(false)), "{path}");
                for (name, child) in properties {
                    assert_strict(child, &format!("{path}.{name}"), used);
                }
            }
            Some("array") => assert_strict(&map["items"], &format!("{path}[]"), used),
            Some("string") => {}
            other => panic!("{path}: unexpected type {other:?}"),
        }
    }

    #[test]
    fn schema_is_strict_at_every_level() {
        for echo in [false, true] {
            let mut used = Vec::new();
            assert_strict(&schema(echo), "$", &mut used);
            // Only keywords both OpenAI strict mode and Gemini's schema subset accept.
            let allowed = [
                "type",
                "properties",
                "required",
                "additionalProperties",
                "items",
                "description",
                "minItems",
                "maxItems",
            ];
            for keyword in used
                .iter()
                .filter(|k| !FIELDS.iter().any(|f| f.0 == k.as_str()) && *k != "candidates" && *k != "echo_note")
            {
                assert!(allowed.contains(&keyword.as_str()), "unsupported schema keyword {keyword}");
            }
        }
    }

    #[test]
    fn schema_leaves_are_strings_or_string_arrays() {
        let schema = schema(true);
        let item = &schema["properties"]["candidates"]["items"];
        for (name, property) in item["properties"].as_object().unwrap() {
            match property["type"].as_str() {
                Some("string") => {}
                Some("array") => assert_eq!(property["items"], json!({"type": "string"}), "{name}"),
                other => panic!("{name}: {other:?}"),
            }
        }
        assert_eq!(schema["properties"]["candidates"]["maxItems"], json!(CANDIDATES));
    }

    #[test]
    fn schema_has_echo_note_only_for_echoes() {
        let has_note =
            |echo| schema(echo)["properties"]["candidates"]["items"]["properties"].get("echo_note").is_some();
        assert!(has_note(true));
        assert!(!has_note(false));
        let item = schema(false)["properties"]["candidates"]["items"].clone();
        let required: Vec<&str> = item["required"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
        // Generation order: plan, keywords, prompt, then title and summary.
        assert_eq!(required.first(), Some(&"setting"));
        assert_eq!(&required[required.len() - 3..], ["prompt", "title", "summary"]);
    }

    #[test]
    fn schema_matches_every_concept_field() {
        // Every Concept field is in the schema, so a schema-valid candidate fills a whole Concept.
        let concept = serde_json::to_value(Concept::default()).unwrap();
        let properties = &schema(false)["properties"]["candidates"]["items"]["properties"];
        for key in concept.as_object().unwrap().keys() {
            assert!(properties.get(key).is_some(), "schema lacks {key}");
        }
    }

    // ── numbers ──

    #[test]
    fn temperature_spans_point_four_to_one_point_two() {
        assert!((temperature(0.0) - 0.4).abs() < 1e-6);
        assert!((temperature(0.5) - 0.8).abs() < 1e-6);
        assert!((temperature(1.0) - 1.2).abs() < 1e-6);
        assert!((temperature(-3.0) - 0.4).abs() < 1e-6);
        assert!((temperature(7.0) - 1.2).abs() < 1e-6);
        assert!((temperature(f32::NAN) - 0.4).abs() < 1e-6);
    }

    #[test]
    fn wildcard_limit_boundaries() {
        let cases = [
            (0.0, 0),
            (0.249, 0),
            (0.25, 1),
            (0.499, 1),
            (0.5, 2),
            (0.749, 2),
            (0.75, 3),
            (1.0, 3),
            (-1.0, 0),
            (9.0, 3),
            (f32::NAN, 0),
        ];
        for (surprise, limit) in cases {
            assert_eq!(wildcard_limit(surprise), limit, "surprise {surprise}");
        }
    }

    #[test]
    fn surprise_bands_follow_the_same_thresholds() {
        use SurpriseBand::*;
        let cases = [
            (0.0, Faithful),
            (0.249, Faithful),
            (0.25, Fresh),
            (0.35, Fresh),
            (0.499, Fresh),
            (0.5, Adventurous),
            (0.749, Adventurous),
            (0.75, Wild),
            (1.0, Wild),
            (-1.0, Faithful),
            (9.0, Wild),
            (f32::NAN, Faithful),
        ];
        for (surprise, band) in cases {
            assert_eq!(surprise_band(surprise), band, "surprise {surprise}");
        }
    }

    #[test]
    fn score_weights_by_surprise() {
        assert!((score(0.8, 0.2, 0.0, 0.9) - 1.0).abs() < 1e-6);
        // s = 1: 0.7 n + 0.4 t + 0.3 j.
        assert!((score(1.0, 0.5, 1.0, 0.5) - (0.7 + 0.2 + 0.15)).abs() < 1e-6);
        assert!(score(0.5, 0.0, 0.5, 0.9) > score(0.5, 0.0, 0.5, 0.1));
        assert_eq!(score(0.5, 0.1, 0.0, 0.0), score(0.5, 0.1, 0.0, 0.99));
        assert!((score(0.5, 0.0, 2.0, 0.0) - score(0.5, 0.0, 1.0, 0.0)).abs() < 1e-6);
    }

    // ── build_request ──

    #[test]
    fn fills_inputs_from_the_context() {
        let mut ctx = context(&["rain", "ruins"], &["blue", "lanterns"], &["people"], 0.35);
        ctx.keywords.reverse(); // order comes from `position`, not the vector
        let request = build_request(&ctx, "gpt-6-luna");
        assert_eq!(request.model, "gpt-6-luna");
        assert_eq!(request.inputs.musts, ["rain", "ruins"]);
        assert_eq!(request.inputs.maybes, ["blue", "lanterns"]);
        assert_eq!(request.inputs.avoids, ["people"]);
        assert_eq!(request.inputs.candidates, CANDIDATES);
        assert!((request.inputs.surprise - 0.35).abs() < 1e-6);
        assert_eq!(request.inputs.echo_of, None);
        assert!((request.temperature - temperature(0.35)).abs() < 1e-6);
        assert_eq!(request.schema, schema(false));
    }

    #[test]
    fn user_lists_keywords_by_weight() {
        let request = build_request(&context(&["rain", "blue hour"], &["lanterns"], &["people"], 0.35), "");
        let user = &request.user;
        assert!(user.contains("Compose 4 wallpaper candidates."));
        assert!(user.contains("Must (every candidate includes every one of these):\n- rain\n- blue hour"));
        assert!(user.contains("Maybe (use some; vary them across candidates):\n- lanterns"));
        assert!(user.contains("Avoid (never part of any candidate, never mentioned in any field):\n- people"));
        assert!(user.contains("Language: write title and summary in English (en-US)."));
        assert!(user.contains("always the prompt, in English"));
    }

    #[test]
    fn empty_groups_say_none() {
        let user = build_request(&context(&[], &[], &[], 0.1), "").user;
        assert!(user.contains("Must: none."));
        assert!(user.contains("Maybe: none."));
        assert!(user.contains("Avoid: none."));
        assert!(user.contains("no Must or Maybe keywords right now"));
    }

    #[test]
    fn low_surprise_is_faithful_without_wildcards() {
        let request = build_request(&context(&["rain"], &[], &[], 0.1), "");
        assert!(
            request.user.contains("Surprise: 0.10 on a scale from 0 (faithful) to 1 (wild), so faithful and literal")
        );
        assert!(request.user.contains("Wildcards: none."));
        assert!(!request.user.contains("loose inspiration"));
    }

    #[test]
    fn middle_surprise_allows_some_wildcards() {
        assert!(build_request(&context(&["rain"], &[], &[], 0.3), "").user.contains("fresh but believable"));
        assert!(
            build_request(&context(&["rain"], &[], &[], 0.3), "").user.contains("Wildcards: at most 1 per candidate.")
        );
        assert!(
            build_request(&context(&["rain"], &[], &[], 0.6), "").user.contains("Wildcards: at most 2 per candidate.")
        );
    }

    #[test]
    fn high_surprise_is_loose_inspiration() {
        let request = build_request(&context(&["rain"], &[], &[], 0.9), "");
        assert!(request.user.contains("so wild: the keywords are loose inspiration"));
        assert!(request.user.contains("Must keywords are still named in the prompt"));
        assert!(request.user.contains("Wildcards: at most 3 per candidate."));
    }

    #[test]
    fn system_states_the_wallpaper_rules() {
        let system = build_request(&context(&["rain"], &[], &[], 0.5), "").system;
        for phrase in [
            "desktop wallpaper",
            "A landscape image, wider than tall",
            "no text, letters, numbers, signs, logos, watermarks, signatures, user-interface elements, frames or borders",
            "Calm where the icons sit",
            "Easy to live with for hours",
            "every candidate includes every Must keyword",
            "not even negated",
            "image models treat any mention as a request",
            "Wildcards are things nobody asked for, added for interest: a distinct object, creature, structure or place",
            "Scene-setting (light, sky, ground, weather and palette) doesn't count as a wildcard",
            "vary the medium too (photograph, painting, illustration or render)",
            "(open negative space, soft gradients, gentle texture and shallow focus)",
            "make them genuinely different ideas",
            "at most 60 characters",
            "for someone who can't see it",
            "Always in English, whatever the person's language",
            "60 to 120 words",
            "No negations of any kind",
            "keywords_used: the person's Must and Maybe keywords this candidate uses",
            "with exactly 4 candidates",
        ] {
            assert!(system.contains(phrase), "system lacks {phrase:?}");
        }
        assert!(!system.contains("## Echoes"));
    }

    #[test]
    fn system_is_stable_across_calls_with_the_same_avoids() {
        let a = build_request(&context(&["rain"], &["moss"], &["people"], 0.1), "a").system;
        let mut other = context(&["fog", "ruins"], &[], &["people"], 0.9);
        other.recent = vec!["Something recent.".into()];
        other.locale = "fr-FR".into();
        assert_eq!(a, build_request(&other, "b").system);
    }

    #[test]
    fn locale_names_the_language() {
        let mut ctx = context(&["rain"], &[], &[], 0.3);
        for (locale, label) in [
            ("de-DE", "German (de-DE)"),
            ("de_AT.UTF-8", "German (de-AT)"),
            ("pt-BR", "Brazilian Portuguese (pt-BR)"),
            ("zh-Hant-TW", "Traditional Chinese (zh-Hant-TW)"),
            ("zh-CN", "Simplified Chinese (zh-CN)"),
            ("ja", "Japanese (ja)"),
            ("xx-YY", "the language of locale xx-YY"),
            ("", "English"),
        ] {
            ctx.locale = locale.into();
            let user = build_request(&ctx, "").user;
            assert!(user.contains(&format!("write title and summary in {label}.")), "{locale}: {user}");
        }
    }

    #[test]
    fn taste_recent_and_too_close_are_named() {
        let mut ctx = context(&["rain"], &[], &[], 0.3);
        ctx.liked = vec!["mist mountain".into(), "  ".into(), "watercolour".into()];
        ctx.disliked = vec!["neon".into()];
        ctx.recent = (0..30).map(|i| format!("Recent scene number {i}.")).collect();
        ctx.too_close = vec!["A misty harbour at dawn\nwith boats.".into()];
        let user = build_request(&ctx, "").user;
        assert!(user.contains("soft preferences"));
        assert!(user.contains("- Tends to like: mist mountain; watercolour"));
        assert!(user.contains("- Tends to dislike: neon"));
        assert!(user.contains("Don't repeat these ideas"));
        assert!(user.contains("1. Recent scene number 0."));
        assert!(user.contains("20. Recent scene number 19."));
        assert!(!user.contains("Recent scene number 20."), "recent is capped at {MAX_RECENT}");
        assert!(user.contains("too close to these past wallpapers. Go somewhere clearly different"));
        assert!(user.contains("- A misty harbour at dawn with boats."));
    }

    #[test]
    fn a_retry_says_what_made_the_last_candidates_unusable() {
        let mut ctx = context(&["lighthouse"], &[], &["people"], 0.3);
        ctx.corrections = vec![
            correction(&Problem::MissingMust("lighthouse".into())),
            correction(&Problem::MentionsAvoid("people".into())),
        ];
        let user = build_request(&ctx, "").user;
        assert!(user.contains("Your previous candidates couldn't be used."), "{user}");
        assert!(user.contains("- The prompt left out the Must keyword \u{201c}lighthouse\u{201d}"), "{user}");
        assert!(user.contains("- A field mentioned the Avoid keyword \u{201c}people\u{201d}"), "{user}");
    }

    #[test]
    fn first_attempt_has_no_retry_text() {
        let user = build_request(&context(&["rain"], &[], &[], 0.3), "").user;
        assert!(!user.contains("too close"));
        assert!(!user.contains("couldn't be used"));
        assert!(!user.contains("Recent wallpapers"));
        assert!(!user.contains("Taste"));
    }

    /// The user message with its Avoid list removed.
    fn without_avoid_list(user: &str) -> String {
        let mut out = Vec::new();
        let mut in_avoid = false;
        for line in user.lines() {
            if line.starts_with("Avoid (") {
                in_avoid = true;
                continue;
            }
            if in_avoid && line.starts_with("- ") {
                continue;
            }
            in_avoid = false;
            out.push(line);
        }
        out.join("\n")
    }

    /// Plausible Avoid keywords, including words the instructions' own lists and examples use. (Rule
    /// vocabulary such as "text", "logos" or "light" is exempt: the rules can't be written without it.)
    const COMMON_AVOIDS: [&str; 52] = [
        "rain",
        "fog",
        "mist",
        "water",
        "ocean",
        "sea",
        "lighthouse",
        "mountains",
        "night",
        "snow",
        "people",
        "boats",
        "trees",
        "city",
        "clouds",
        "sun",
        "moon",
        "birds",
        "forest",
        "desert",
        "red",
        "blue",
        "gold",
        "neon",
        "cars",
        "flowers",
        "castle",
        "dragons",
        "glass",
        "lamps",
        "creatures",
        "painting",
        "photographs",
        "sky",
        "ground",
        "windows",
        "buildings",
        "animals",
        "statues",
        "fire",
        "smoke",
        "skulls",
        "insects",
        "stars",
        "storms",
        "waves",
        "rocks",
        "ruins",
        "towers",
        "bridges",
        "roads",
        "horses",
    ];

    #[test]
    fn the_computers_appearance_is_asked_for_only_when_known() {
        let plain = build_request(&context(&["harbour"], &[], &[], 0.3), "").user;
        assert!(!plain.contains("light mode") && !plain.contains("dark mode"), "{plain}");
        for (appearance, word) in [(Appearance::Light, "light mode"), (Appearance::Dark, "dark mode")] {
            let user = build_request(&ComposeContext { appearance: Some(appearance), ..context(&["harbour"], &[], &[], 0.3) }, "").user;
            assert!(user.contains(word), "{user}");
        }
    }

    #[test]
    fn a_gentler_retry_says_so_and_a_new_medium_is_named() {
        let plain = build_request(&context(&["harbour"], &[], &[], 0.3), "");
        assert!(!plain.user.contains("declined the previous attempt"));
        let gentle = build_request(&ComposeContext { gentler: true, ..context(&["harbour"], &[], &[], 0.3) }, "");
        assert!(gentle.user.contains("declined the previous attempt"), "{}", gentle.user);
        assert_eq!(gentle.system, plain.system, "the system instructions stay cacheable");

        let medium = build_request(&echo_context(vec![EchoAxis::Medium, EchoAxis::Weather], &[], 0.3), "");
        assert!(medium.user.contains("Name the new medium"), "{}", medium.user);
        let other = build_request(&echo_context(vec![EchoAxis::Season, EchoAxis::Weather], &[], 0.3), "");
        assert!(!other.user.contains("Name the new medium"));
        // The added rule text puts no plausible Avoid keyword in front of the model.
        let added: Vec<&str> = gentle
            .user
            .lines()
            .filter(|line| line.contains("declined the previous attempt"))
            .chain(medium.user.lines().filter(|line| line.contains("Name the new medium")))
            .collect();
        assert_eq!(added.len(), 2);
        for avoid in COMMON_AVOIDS {
            assert!(!added.iter().any(|line| text::mentions(line, avoid)), "mentions {avoid:?}");
        }
    }

    #[test]
    fn avoid_keywords_appear_only_in_the_avoid_list() {
        // One at a time and all together: no example or instruction text written here mentions them.
        let mut sets: Vec<Vec<&str>> = COMMON_AVOIDS.iter().map(|a| vec![*a]).collect();
        sets.push(COMMON_AVOIDS.to_vec());
        for avoids in sets {
            for surprise in [0.1, 0.9] {
                let request = build_request(&context(&["harbour"], &["lanterns"], &avoids, surprise), "");
                let user = without_avoid_list(&request.user);
                for avoid in &avoids {
                    assert!(!text::mentions(&request.system, avoid), "system mentions {avoid:?}");
                    assert!(!text::mentions(&user, avoid), "user mentions {avoid:?} outside the Avoid list");
                }
            }
        }
    }

    #[test]
    #[ignore = "prints the rendered instructions for review"]
    fn dump_instructions() {
        let mut ctx = context(&["rain", "ruins"], &["blue", "lanterns", "moss"], &["people"], 0.35);
        ctx.locale = "de-DE".into();
        ctx.liked = vec!["mist mountain".into(), "watercolour".into()];
        ctx.disliked = vec!["neon".into()];
        ctx.recent = vec!["Ein Leuchtturm im Nebel.".into(), "Terraced tea fields at dusk.".into()];
        let request = build_request(&ctx, "");
        println!("=== SYSTEM ===\n{}\n=== USER ===\n{}", request.system, request.user);
        let mut echo = echo_context(vec![EchoAxis::Weather, EchoAxis::Viewpoint], &["people"], 0.6);
        echo.too_close = vec!["Silver towers at dawn.".into()];
        let request = build_request(&echo, "");
        let echo_part = request.system.split("## Echoes").nth(1).unwrap_or_default();
        println!("=== ECHO SYSTEM TAIL ===\n## Echoes{echo_part}\n=== ECHO USER ===\n{}", request.user);
        let rules =
            build_request(&context(&[], &[], &["lighthouse", "tea", "desert", "snow", "greenhouse"], 0.5), "").system;
        let mut stems: Vec<String> = text::words(&rules).iter().map(|w| text::stem(w)).collect();
        stems.sort();
        stems.dedup();
        println!("=== RULE STEMS ===\n{}", stems.join(" "));
        println!("=== SCHEMA ===\n{}", serde_json::to_string_pretty(&schema(true)).unwrap_or_default());
    }

    #[test]
    fn examples_are_shown_and_filtered() {
        let system = build_request(&context(&["rain"], &[], &[], 0.3), "").system;
        assert!(system.contains("## Example prompts"));
        assert!(system.contains("lighthouse"));
        // Avoiding the first example's subject brings in another.
        let system = build_request(&context(&["rain"], &[], &["lighthouse"], 0.3), "").system;
        assert!(system.contains("## Example prompts"));
        assert!(!system.contains("lighthouse"));
        assert_eq!(system.matches("\n- ").count() - base_bullets(&system), EXAMPLES_SHOWN);
        // When every example is avoided, the section is left out rather than shown unfiltered.
        let all = ["lighthouse", "tea", "desert", "snow", "greenhouse"];
        let system = build_request(&context(&["rain"], &[], &all, 0.3), "").system;
        assert!(!system.contains("## Example prompts"));
    }

    fn base_bullets(system: &str) -> usize {
        let before_examples = system.split("## Example prompts").next().unwrap_or(system);
        before_examples.matches("\n- ").count()
    }

    #[test]
    fn every_example_prompt_follows_the_prompt_rules() {
        for example in EXAMPLE_PROMPTS {
            let words = example.split_whitespace().count();
            assert!((60..=120).contains(&words), "{words} words: {example}");
            for negation in ["no", "not", "without", "free"] {
                assert!(!text::words(example).iter().any(|w| w == negation), "{negation:?} in {example}");
            }
        }
    }

    #[test]
    fn illustrative_lists_drop_avoided_items() {
        let system = build_request(&context(&[], &[], &["creatures", "painting", "sky"], 0.5), "").system;
        assert!(system.contains("added for interest: a distinct object, structure or place."));
        assert!(system.contains("Scene-setting (light, ground, weather and palette)"));
        assert!(system.contains("vary the medium too (photograph, illustration or render)."));
        let all: Vec<&str> = WILDCARD_KINDS.iter().chain(&SCENE_SETTING).chain(&MEDIA).copied().collect();
        let system = build_request(&context(&[], &[], &all, 0.5), "").system;
        assert!(system.contains("Wildcards are things nobody asked for, added for interest. Scene-setting doesn't"));
        assert!(system.contains("vary the medium too. Each"));
    }

    #[test]
    fn inflection_examples_match_the_checker() {
        for (form, base) in INFLECTIONS {
            assert!(text::mentions(form, base), "{form} / {base}");
        }
        let system = build_request(&context(&["rain"], &[], &["rain"], 0.3), "").system;
        assert!(system.contains("such as \"stormy\" for \"storm\""));
        let all: Vec<&str> = INFLECTIONS.iter().map(|(_, base)| *base).collect();
        let system = build_request(&context(&["rain"], &[], &all, 0.3), "").system;
        assert!(system.contains("(an inflected form counts)"));
    }

    #[test]
    fn maybes_and_hints_that_mention_an_avoid_are_dropped() {
        let mut ctx = context(&["harbour"], &["red fox", "lanterns"], &["red"], 0.3);
        ctx.liked = vec!["red sails".into(), "watercolour".into()];
        ctx.disliked = vec!["red roofs".into()];
        let request = build_request(&ctx, "");
        assert_eq!(request.inputs.maybes, ["lanterns"]);
        assert!(!text::mentions(&without_avoid_list(&request.user), "red"));
        assert!(request.user.contains("- Tends to like: watercolour"));
        assert!(!request.user.contains("Tends to dislike"));
    }

    #[test]
    fn echo_request_describes_the_original_and_the_changes() {
        let ctx = echo_context(vec![EchoAxis::Weather, EchoAxis::TimeOfDay], &["people"], 0.3);
        let request = build_request(&ctx, "");
        let user = &request.user;
        assert!(user.contains("Compose 4 echo candidates."));
        assert!(user.contains("The original wallpaper, first shown 2 years ago:"));
        assert!(user.contains("- Title: Black ocean, silver structures"));
        assert!(user.contains("- Summary: Tall silver towers rise from a black ocean under a pale moon."));
        assert!(user.contains("- Elements: silver towers; black water; pale moon"));
        assert!(user.contains("- Its image prompt: Photograph of tall silver towers"));
        assert!(user.contains("Keep its essence (the same kind of place and its key elements)"));
        assert!(user.contains(&format!("- {}", EchoAxis::Weather.instruction())));
        assert!(user.contains(&format!("- {}", EchoAxis::TimeOfDay.instruction())));
        assert!(
            user.contains(
                "echo_note, in the person's language: Echo of “Black ocean, silver structures” (2 years ago)"
            )
        );
        assert!(user.contains("write title, summary and echo_note in German (de-DE)"));
        assert!(user.contains("For this echo it sets how far the changes go: make the changes clear"));
        assert!(!user.contains("fresh but believable"), "keyword-reading bands don't apply to echoes");
        assert!(user.contains("even where the original had it):\n- people"));
        // Musts and Maybes don't apply to echoes.
        assert!(!user.contains("lanterns"));
        assert!(!user.contains("moss"));
        assert!(request.system.contains("## Echoes"));
        assert!(request.system.contains("Keep the original's essence"));
        assert!(request.system.contains("Must and Maybe keywords don't apply to echoes; Avoid still does"));
        assert_eq!(request.schema, schema(true));
        assert_eq!(request.inputs.echo_of, Some(original()));
        assert!(request.inputs.musts.is_empty());
        assert!(request.inputs.maybes.is_empty());
        assert_eq!(request.inputs.avoids, ["people"]);
    }

    #[test]
    fn echo_axis_examples_never_suggest_an_avoid() {
        let request =
            build_request(&echo_context(vec![EchoAxis::Weather, EchoAxis::PassageOfTime], &["snow", "ruins"], 0.8), "");
        assert!(request.user.contains("- Change the weather.\n"));
        assert!(request.user.contains("- Let years pass in the scene.\n"));
        let instructions: Vec<&str> =
            request.user.lines().filter(|l| l.starts_with("- Change") || l.starts_with("- Let")).collect();
        for line in instructions {
            assert!(!text::mentions(line, "snow") && !text::mentions(line, "ruins"), "{line}");
        }
        assert!(!text::mentions(&request.system, "snow"));
        assert!(request.user.contains("push every change far"));
    }

    #[test]
    fn retry_echo_keeps_the_too_close_list() {
        let mut ctx = echo_context(vec![EchoAxis::Season], &[], 0.3);
        ctx.too_close = vec!["Silver towers in a black ocean at night.".into()];
        ctx.recent = vec!["A harbour in fog.".into()];
        let user = build_request(&ctx, "").user;
        assert!(user.contains("too close to these other past wallpapers. Keep the original's essence"));
        assert!(!user.contains("another kind of place"), "an echo keeps its kind of place");
        assert!(user.contains("- Silver towers in a black ocean at night."));
        assert!(user.contains("Apart from the original, don't repeat these:\n1. A harbour in fog."));
    }

    // ── parse ──

    #[test]
    fn parses_the_schema_shape() {
        let output = json!({ "candidates": [candidate_json("A harbour with boats."), candidate_json("Another.")] });
        let parsed = parse(&output, false).unwrap();
        assert_eq!(parsed.len(), 2);
        let first = &parsed[0];
        assert_eq!(first.concept.title, "Harbour at dawn");
        assert_eq!(first.concept.elements, ["boats", "pier"]);
        assert_eq!(first.concept.prompt, "A harbour with boats.");
        assert_eq!(first.echo_note, None);
    }

    #[test]
    fn parses_a_json_string_and_a_fenced_one() {
        let object = json!({ "candidates": [candidate_json("A harbour.")] });
        let as_string = Value::String(object.to_string());
        assert_eq!(parse(&as_string, false).unwrap().len(), 1);
        let fenced = Value::String(format!("```json\n{object}\n```"));
        assert_eq!(parse(&fenced, false).unwrap().len(), 1);
        let one_line_fence = Value::String(format!("```{object}```"));
        assert_eq!(parse(&one_line_fence, false).unwrap().len(), 1);
        let chatty = Value::String(format!("Here are your candidates:\n{object}\nEnjoy!"));
        assert_eq!(parse(&chatty, false).unwrap().len(), 1);
        let double = Value::String(Value::String(object.to_string()).to_string());
        assert_eq!(parse(&double, false).unwrap().len(), 1);
    }

    #[test]
    fn parses_a_bare_array_and_a_single_candidate() {
        let array = json!([candidate_json("One."), candidate_json("Two."), candidate_json("Three.")]);
        assert_eq!(parse(&array, false).unwrap().len(), 3);
        assert_eq!(parse(&candidate_json("Solo."), false).unwrap()[0].concept.prompt, "Solo.");
        let stringy_items = json!({ "candidates": [candidate_json("One.").to_string()] });
        assert_eq!(parse(&stringy_items, false).unwrap().len(), 1);
    }

    #[test]
    fn missing_arrays_become_empty_and_strings_are_trimmed() {
        let output = json!({ "candidates": [{
            "title": "  “Quiet harbour”  ", "summary": " Boats at rest. ", "prompt": "\n Harbour at dawn. \n",
            "elements": ["  boats ", "", 7, null, {"x": 1}], "season": null, "weather": 12
        }] });
        let parsed = parse(&output, false).unwrap();
        let c = &parsed[0].concept;
        assert_eq!(c.title, "Quiet harbour");
        assert_eq!(c.summary, "Boats at rest.");
        assert_eq!(c.prompt, "Harbour at dawn.");
        assert_eq!(c.elements, ["boats", "7"]);
        assert!(c.mood.is_empty() && c.palette.is_empty() && c.keywords_used.is_empty() && c.wildcards.is_empty());
        assert_eq!(c.season, "");
        assert_eq!(c.weather, "12");
    }

    #[test]
    fn control_characters_become_spaces() {
        let output = json!({ "candidates": [{
            "title": "Harbour\u{1b}[2J at dawn", "summary": "Boats\u{1b}]52;c;QUJD\u{7} at rest.\r",
            "prompt": "Harbour\tat dawn.", "elements": ["boats\u{1b}[31m", "pier"], "palette": "slate\u{0}grey, rose",
            "echo_note": "After\u{8}\u{8} rain"
        }] });
        let parsed = parse(&output, true).unwrap();
        let c = &parsed[0].concept;
        assert_eq!(c.title, "Harbour [2J at dawn");
        assert_eq!(c.summary, "Boats ]52;c;QUJD  at rest.");
        assert_eq!(c.prompt, "Harbour at dawn.");
        assert_eq!(c.elements, ["boats [31m", "pier"]);
        assert_eq!(c.palette, ["slate grey", "rose"]);
        assert_eq!(parsed[0].echo_note.as_deref(), Some("After   rain"));
    }

    #[test]
    fn lists_given_as_strings_are_split_and_arrays_capped() {
        let mut candidate = candidate_json("A harbour.");
        candidate["palette"] = json!("slate grey, rose; pearl\nwhite");
        candidate["elements"] = json!((0..20).map(|i| format!("thing {i}")).collect::<Vec<_>>());
        candidate["style"] = json!(["photograph", "long exposure"]);
        let parsed = parse(&json!({ "candidates": [candidate] }), false).unwrap();
        let c = &parsed[0].concept;
        assert_eq!(c.palette, ["slate grey", "rose", "pearl", "white"]);
        assert_eq!(c.elements.len(), MAX_ITEMS);
        assert_eq!(c.style, "photograph, long exposure");
    }

    #[test]
    fn candidates_are_capped() {
        let many: Vec<Value> = (0..30).map(|i| candidate_json(&format!("Prompt {i}."))).collect();
        assert_eq!(parse(&json!({ "candidates": many }), false).unwrap().len(), MAX_ITEMS);
    }

    #[test]
    fn long_titles_are_shortened_at_a_word() {
        let mut candidate = candidate_json("A harbour.");
        candidate["title"] = json!("A very long title about a quiet harbour at dawn with boats and gulls and mist");
        let title = parse(&candidate, false).unwrap()[0].concept.title.clone();
        assert!(title.chars().count() <= MAX_TITLE_CHARS, "{title}");
        assert!(title.ends_with('…'));
        assert_eq!(title, "A very long title about a quiet harbour at dawn with boats…");
        let mut unbroken = candidate_json("A harbour.");
        unbroken["title"] = json!("x".repeat(80));
        assert_eq!(parse(&unbroken, false).unwrap()[0].concept.title.chars().count(), MAX_TITLE_CHARS);
    }

    #[test]
    fn echo_note_only_for_echoes() {
        let mut candidate = candidate_json("A harbour.");
        candidate["echo_note"] = json!("  Echo of “Harbour” (2 years ago): in snow.  ");
        assert_eq!(
            parse(&candidate, true).unwrap()[0].echo_note.as_deref(),
            Some("Echo of “Harbour” (2 years ago): in snow.")
        );
        assert_eq!(parse(&candidate, false).unwrap()[0].echo_note, None);
        assert_eq!(parse(&candidate_json("A harbour."), true).unwrap()[0].echo_note, None);
    }

    #[test]
    fn unreadable_output_is_an_invalid_response() {
        for output in [
            json!({}),
            json!({ "candidates": [] }),
            json!({ "candidates": [1, "two", null, {}, {"unrelated": true}] }),
            json!({ "answer": "Sorry, I can't help with that." }),
            json!("not json at all"),
            json!(42),
            Value::Null,
            json!([]),
        ] {
            assert!(
                matches!(parse(&output, false), Err(AutoPaperError::InvalidResponse { .. })),
                "{output} should be unreadable"
            );
        }
    }

    #[test]
    fn readable_candidates_survive_unreadable_neighbours() {
        let output = json!({ "candidates": [42, {"nope": 1}, candidate_json("Good.")] });
        let parsed = parse(&output, false).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].concept.prompt, "Good.");
    }

    // ── check ──

    #[test]
    fn a_good_candidate_has_no_problems() {
        let kws = keywords(&["rain", "blue hour"], &["lanterns"], &["people"]);
        let c = concept(
            "Ancient ruins in the rainy blue hours, lanterns glowing.",
            "Rain at the ruins",
            "Ruins glisten in rain.",
        );
        assert_eq!(check(&c, &kws, false), Vec::<Problem>::new());
    }

    #[test]
    fn a_missing_must_is_reported_in_keyword_order() {
        let kws = keywords(&["rain", "ruins", "blue hour"], &["lanterns"], &[]);
        let c = concept("Ruins at dusk under a blue sky, an hour of quiet.", "Ruins", "Old ruins.");
        assert_eq!(
            check(&c, &kws, false),
            [Problem::MissingMust("rain".into()), Problem::MissingMust("blue hour".into())]
        );
    }

    #[test]
    fn a_must_in_title_or_summary_does_not_count() {
        let kws = keywords(&["rain"], &[], &[]);
        let c = concept("Ruins at dusk.", "Rain", "Ruins in the rain.");
        assert_eq!(check(&c, &kws, false), [Problem::MissingMust("rain".into())]);
    }

    #[test]
    fn echoes_skip_musts_but_not_avoids() {
        let kws = keywords(&["rain"], &[], &["people"]);
        let c = concept("Silver towers in a black ocean at sunrise.", "Towers at sunrise", "Silver towers at dawn.");
        assert!(check(&c, &kws, true).is_empty());
        let crowded = concept("Silver towers, people on the shore.", "Towers", "Towers.");
        assert_eq!(check(&crowded, &kws, true), [Problem::MentionsAvoid("people".into())]);
    }

    #[test]
    fn avoid_is_caught_in_prompt_title_and_summary_by_stem() {
        let kws = keywords(&[], &[], &["rain"]);
        let negated = concept("A clear harbour with no rain.", "Harbour", "A harbour.");
        assert_eq!(check(&negated, &kws, false), [Problem::MentionsAvoid("rain".into())]);
        let in_title = concept("A clear harbour.", "Rainy harbour", "A harbour.");
        assert_eq!(check(&in_title, &kws, false), [Problem::MentionsAvoid("rain".into())]);
        let in_summary = concept("A clear harbour.", "Harbour", "A harbour after raining all day.");
        assert_eq!(check(&in_summary, &kws, false), [Problem::MentionsAvoid("rain".into())]);
        // Elsewhere (not shown or sent) it isn't checked; a different word with the same letters isn't a match.
        let mut elsewhere = concept("A clear harbour, a rainbow.", "Harbour", "A harbour.");
        elsewhere.elements = vec!["rain".into()];
        assert!(check(&elsewhere, &kws, false).is_empty());
    }

    #[test]
    fn each_avoid_is_reported_once() {
        let kws = keywords(&[], &[], &["rain", "fog"]);
        let c = concept("Rain and fog over rain.", "Rain", "Foggy rain.");
        assert_eq!(
            check(&c, &kws, false),
            [Problem::MentionsAvoid("rain".into()), Problem::MentionsAvoid("fog".into())]
        );
    }

    #[test]
    fn empty_fields_and_long_prompts() {
        let kws = keywords(&["rain"], &[], &[]);
        let empty = concept("  ", "", "\n");
        assert_eq!(
            check(&empty, &kws, false),
            [
                Problem::EmptyField("title"),
                Problem::EmptyField("summary"),
                Problem::EmptyField("prompt"),
                Problem::MissingMust("rain".into()),
            ]
        );
        // Characters, not bytes: 2500 multi-byte characters fit, 2501 don't.
        let fits = concept(&format!("rain {}", "é".repeat(MAX_PROMPT_CHARS - 5)), "T", "S");
        assert!(check(&fits, &kws, false).is_empty());
        let long = concept(&format!("rain {}", "é".repeat(MAX_PROMPT_CHARS - 4)), "T", "S");
        assert_eq!(check(&long, &kws, false), [Problem::PromptTooLong]);
    }

    #[test]
    fn maybes_are_never_required() {
        let kws = keywords(&[], &["lanterns", "moss"], &[]);
        assert!(check(&concept("A quiet harbour.", "Harbour", "A harbour."), &kws, false).is_empty());
    }

    #[test]
    fn parsed_candidates_round_trip_through_check() {
        let kws = keywords(&["boats"], &[], &["people"]);
        let parsed =
            parse(&json!({ "candidates": [candidate_json("Photograph of fishing boats in a misty harbour.")] }), false)
                .unwrap();
        assert!(check(&parsed[0].concept, &kws, false).is_empty());
    }

    // ── helpers ──

    #[test]
    fn shorten_and_one_line() {
        assert_eq!(shorten("short", 10), "short");
        assert_eq!(shorten("two words", 5), "two…");
        assert_eq!(shorten("abcdefgh", 4), "abc…");
        assert_eq!(shorten("anything", 0), "");
        assert_eq!(one_line("  a\n\tb \u{7} c  ", 100), "a b c");
    }
}
