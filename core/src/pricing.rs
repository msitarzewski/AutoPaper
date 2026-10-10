//! Estimated costs, in micro-US-dollars, from a versioned table of published prices
//! (`docs/research/providers.md`, verified 2026-10-05). Estimates only — the UI says so. Local providers
//! and Demo cost 0. Unknown models fall back to their provider's default model's price (an estimate is
//! better than none; the UI still labels it estimated).

use crate::model::{ImageQuality, ProviderKind};
use crate::providers::Usage;
use crate::providers::{google, openai};

/// When the table was last checked against the providers' pricing pages.
pub const PRICES_AS_OF: &str = "2026-10-05";

/// `PRICES_AS_OF` for hosts ("YYYY-MM-DD"): Settings → Budget says "Prices as of <date>" in the host's date format.
#[uniffi::export]
pub fn prices_as_of() -> String {
    PRICES_AS_OF.to_string()
}

/// A typical compose call (instructions + 4 candidates), for estimates before a call is made.
pub const TYPICAL_COMPOSE_USAGE: Usage = Usage { input_tokens: 1800, output_tokens: 1600 };

/// Cost of one image at this size and quality. OpenAI gpt-image-2.5 models (and gpt-image-2): image
/// output tokens × $30/1M using the docs' calculator formula (Standard = "medium", High = "high").
/// Legacy gpt-image-1.x models: their published per-image High price (square or not); the research
/// recorded no other quality, so Standard is estimated at that upper bound. Gemini image models:
/// per-resolution-tier price, the tier (0.5K/1K/2K/4K) read from the pixel count so that every aspect
/// ratio of a tier lands in it. Local providers, Demo, OpenAI-compatible servers (no published price)
/// and a zero-area size cost 0.
pub fn image_cost(kind: ProviderKind, model: &str, width: u32, height: u32, quality: ImageQuality) -> u64 {
    if width == 0 || height == 0 {
        return 0;
    }
    match kind {
        ProviderKind::OpenAi => openai_image_cost(&model_id(model), width, height, quality),
        ProviderKind::Google => google_image_cost(&model_id(model), width, height),
        ProviderKind::Ollama | ProviderKind::OpenAiCompatible | ProviderKind::ComfyUi | ProviderKind::Demo | ProviderKind::System => 0,
    }
}

/// Cost of a text call from reported token usage (every input token at the uncached price). Rounded up
/// to the next micro-dollar.
pub fn text_cost(kind: ProviderKind, model: &str, usage: &Usage) -> u64 {
    let price = match kind {
        ProviderKind::OpenAi => lookup(OPENAI_TEXT, &model_id(model), openai::DEFAULT_TEXT_MODEL),
        ProviderKind::Google => lookup(GOOGLE_TEXT, &model_id(model), google::DEFAULT_TEXT_MODEL),
        ProviderKind::Ollama | ProviderKind::OpenAiCompatible | ProviderKind::ComfyUi | ProviderKind::Demo | ProviderKind::System => None,
    };
    let Some(price) = price else { return 0 };
    let micro_millionths = u128::from(usage.input_tokens) * u128::from(price.input)
        + u128::from(usage.output_tokens) * u128::from(price.output);
    u64::try_from(micro_millionths.div_ceil(1_000_000)).unwrap_or(u64::MAX)
}

// ── Text prices (micro-USD per 1M tokens, Standard tier) ────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
struct TextPrice {
    model: &'static str,
    input: u64,
    output: u64,
}

const fn per_million(model: &'static str, input: u64, output: u64) -> TextPrice {
    TextPrice { model, input, output }
}

/// providers.md §1.3.
const OPENAI_TEXT: &[TextPrice] = &[
    per_million("gpt-6-luna", 100_000, 500_000),
    per_million("gpt-6.1-sol", 2_000_000, 10_000_000),
    per_million("gpt-6-astra", 10_000_000, 50_000_000),
    per_million("gpt-5.4-nano", 200_000, 1_250_000),
    per_million("gpt-5-nano", 50_000, 400_000),
    per_million("gpt-4o-mini", 150_000, 600_000),
];

/// providers.md §2.4.
const GOOGLE_TEXT: &[TextPrice] = &[
    per_million("gemini-3.5-flash-lite", 300_000, 2_500_000),
    per_million("gemini-3.1-flash-lite", 250_000, 1_500_000),
    // $0.75 / $3.75 is an introductory price until 2026-12-31; the standard price after it is used so
    // the estimate doesn't silently halve for the rest of the table's life.
    per_million("gemini-3.8-flash", 1_500_000, 7_500_000),
];

fn lookup(table: &[TextPrice], model: &str, default_model: &str) -> Option<TextPrice> {
    let find = |id: &str| table.iter().copied().find(|price| is_model(id, price.model));
    find(model).or_else(|| find(default_model))
}

// ── OpenAI images (providers.md §1.4.6) ─────────────────────────────────────────────────────────

/// Image output tokens are $30 per 1M: 30 micro-USD each.
const OPENAI_IMAGE_MICROUSD_PER_TOKEN: u64 = 30;

/// Published per-image High prices of the legacy fixed-size models: (model, 1024×1024, 1536×1024).
const OPENAI_LEGACY_IMAGE: &[(&str, u64, u64)] =
    &[("gpt-image-1.5", 133_000, 200_000), ("gpt-image-1-mini", 36_000, 52_000), ("gpt-image-1", 167_000, 250_000)];

fn openai_image_cost(model: &str, width: u32, height: u32, quality: ImageQuality) -> u64 {
    openai_image_price(model, width, height, quality)
        .or_else(|| openai_image_price(openai::DEFAULT_IMAGE_MODEL, width, height, quality))
        .unwrap_or(0)
}

/// `None` for a model the table doesn't know.
fn openai_image_price(model: &str, width: u32, height: u32, quality: ImageQuality) -> Option<u64> {
    if let Some(&(_, square, other)) = OPENAI_LEGACY_IMAGE.iter().find(|(id, _, _)| is_model(model, id)) {
        return Some(if width == height { square } else { other });
    }
    // The calculator's per-quality base (Standard = medium, High = high): gpt-image-2.5 models
    // {low 16, medium 24, high 48, xhigh 64, max 96}; gpt-image-2 {low 16, medium 48, high 96}.
    let (standard, high) = if model.starts_with("gpt-image-2.5-") {
        (24, 48)
    } else if is_model(model, "gpt-image-2") {
        (48, 96)
    } else {
        return None;
    };
    let base = match quality {
        ImageQuality::Standard => standard,
        ImageQuality::High => high,
    };
    Some(gpt_image_tokens(base, width, height).saturating_mul(OPENAI_IMAGE_MICROUSD_PER_TOKEN))
}

/// OpenAI's calculator, step for step in the same floating-point order so results match it exactly:
/// `s = base / (long/short)`, `u = round-half-even(s)`, `tokens = ceil(base·u · (2e6 + w·h) / 4e6)`.
fn gpt_image_tokens(base: u32, width: u32, height: u32) -> u64 {
    let (w, h) = (f64::from(width), f64::from(height));
    let (long, short) = (w.max(h), w.min(h));
    let s = f64::from(base) / (long / short);
    let grid = f64::from(base) * s.round_ties_even();
    // Saturating float → int conversion; sizes are far below the range where it would matter.
    (grid * (2e6 + w * h) / 4e6).ceil() as u64
}

// ── Gemini images (providers.md §2.3.3) ─────────────────────────────────────────────────────────

/// Per-image prices by tier: [0.5K, 1K, 2K, 4K]. Tiers a model doesn't offer use its nearest one.
const GOOGLE_IMAGE: &[(&str, [u64; 4])] = &[
    ("gemini-3.1-flash-image", [45_000, 67_000, 101_000, 151_000]),
    ("gemini-3-pro-image", [134_000, 134_000, 134_000, 240_000]),
    ("gemini-3.1-flash-lite-image", [33_600, 33_600, 33_600, 33_600]),
];

fn google_image_cost(model: &str, width: u32, height: u32) -> u64 {
    let find = |id: &str| GOOGLE_IMAGE.iter().find(|(name, _)| is_model(id, name)).map(|(_, prices)| *prices);
    let Some(prices) = find(model).or_else(|| find(google::DEFAULT_IMAGE_MODEL)) else { return 0 };
    prices[gemini_tier(width, height)]
}

/// 0 = 0.5K, 1 = 1K, 2 = 2K, 3 = 4K. A tier's sizes share one pixel budget across aspect ratios
/// (1K ≈ 1.05 MP whether 1376×768 or 1024×1024), so the boundaries sit at the geometric midpoints
/// 2^19, 2^21 and 2^23 pixels.
fn gemini_tier(width: u32, height: u32) -> usize {
    let pixels = u64::from(width) * u64::from(height);
    match pixels {
        0..=0x8_0000 => 0,
        0x8_0001..=0x20_0000 => 1,
        0x20_0001..=0x80_0000 => 2,
        _ => 3,
    }
}

// ── Model IDs ───────────────────────────────────────────────────────────────────────────────────

/// Lower-cased and trimmed, without Gemini's "models/" prefix. Empty means the provider's default,
/// which `lookup` and the fallbacks reach because no table entry matches it.
fn model_id(model: &str) -> String {
    let id = model.trim().to_ascii_lowercase();
    match id.strip_prefix("models/") {
        Some(rest) => rest.to_string(),
        None => id,
    }
}

/// `id` is `name`, a dated or numbered snapshot of it (`gpt-6-luna-2026-09-22`, `…-001`), or its
/// `-preview` / `-latest` alias, and not a different model that merely starts the same way
/// (`gpt-image-2.5-flare` is not `gpt-image-2`; `gemini-3.1-flash-lite-image` is not
/// `gemini-3.1-flash-lite`).
fn is_model(id: &str, name: &str) -> bool {
    match id.strip_prefix(name) {
        Some("") => true,
        Some(rest) => rest.strip_prefix('-').is_some_and(|suffix| {
            suffix.starts_with(|c: char| c.is_ascii_digit())
                || suffix.starts_with("preview")
                || suffix.starts_with("latest")
        }),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HIGH: ImageQuality = ImageQuality::High;
    const STANDARD: ImageQuality = ImageQuality::Standard;

    #[test]
    fn hosts_get_the_price_tables_date() {
        assert_eq!(prices_as_of(), PRICES_AS_OF);
        let date = prices_as_of();
        assert!(chrono::NaiveDate::parse_from_str(&date, "%Y-%m-%d").is_ok(), "{date}");
    }

    fn openai(model: &str, w: u32, h: u32, quality: ImageQuality) -> u64 {
        image_cost(ProviderKind::OpenAi, model, w, h, quality)
    }

    fn google(model: &str, w: u32, h: u32) -> u64 {
        image_cost(ProviderKind::Google, model, w, h, HIGH)
    }

    #[test]
    fn gpt_image_2_5_matches_the_research_worked_numbers() {
        // providers.md §1.4.6: 3840×2160 high $0.1001 (3336 tokens, as in the documented response's
        // usage), medium $0.0260 (865 tokens).
        assert_eq!(openai("gpt-image-2.5-flare", 3840, 2160, HIGH), 3336 * 30);
        assert_eq!(openai("gpt-image-2.5-flare", 3840, 2160, HIGH), 100_080);
        assert_eq!(openai("gpt-image-2.5-flare", 3840, 2160, STANDARD), 25_950);
        assert_eq!(openai("gpt-image-2.5-sunburst", 3840, 2160, HIGH), 100_080);
    }

    #[test]
    fn gpt_image_2_5_reproduces_the_research_table() {
        // (w, h, medium $, high $) from the table, to its 4 decimals.
        let table = [
            (1024, 1024, 0.0132, 0.0527),
            (1536, 1024, 0.0103, 0.0412),
            (1920, 1088, 0.0103, 0.0398),
            (2560, 1440, 0.0143, 0.0553),
            (2560, 1600, 0.0165, 0.0659),
            (3440, 1440, 0.0125, 0.0501),
            (3840, 1648, 0.0150, 0.0630),
            (3632, 2272, 0.0277, 0.1107),
            (3840, 2160, 0.0260, 0.1001),
        ];
        for (w, h, medium, high) in table {
            for (quality, dollars) in [(STANDARD, medium), (HIGH, high)] {
                let micro = openai("gpt-image-2.5-flare", w, h, quality) as f64;
                assert!((micro - dollars * 1e6).abs() <= 50.0, "{w}x{h} {quality:?}: {micro} µ$ vs ${dollars}");
            }
        }
    }

    #[test]
    fn portrait_costs_the_same_as_landscape() {
        assert_eq!(openai("", 2160, 3840, HIGH), openai("", 3840, 2160, HIGH));
    }

    #[test]
    fn gpt_image_2_uses_its_own_bases() {
        // §1.4.6: for gpt-image-2, 3840×2160 medium $0.100 and high $0.400; published 1024² high $0.211.
        assert_eq!(openai("gpt-image-2", 3840, 2160, STANDARD), 100_080);
        assert_eq!(openai("gpt-image-2", 3840, 2160, HIGH), 13_342 * 30);
        assert_eq!(openai("gpt-image-2-2026-04-21", 1024, 1024, HIGH), 7024 * 30);
    }

    #[test]
    fn legacy_openai_models_use_published_high_prices() {
        assert_eq!(openai("gpt-image-1", 1024, 1024, HIGH), 167_000);
        assert_eq!(openai("gpt-image-1", 1536, 1024, STANDARD), 250_000);
        assert_eq!(openai("gpt-image-1-mini", 1536, 1024, HIGH), 52_000);
        assert_eq!(openai("gpt-image-1.5", 1024, 1024, HIGH), 133_000);
    }

    #[test]
    fn unknown_or_default_openai_image_model_prices_as_flare() {
        let flare = openai("gpt-image-2.5-flare", 2560, 1440, HIGH);
        assert_eq!(openai("", 2560, 1440, HIGH), flare);
        assert_eq!(openai("chatgpt-image-latest", 2560, 1440, HIGH), flare);
        assert_eq!(openai("  GPT-Image-2.5-Flare  ", 2560, 1440, HIGH), flare);
        assert_eq!(openai("gpt-image-2.5-flare-2026-09-08", 2560, 1440, HIGH), flare);
    }

    #[test]
    fn nano_banana_2_tiers() {
        // §2.3.2 sizes per tier, all aspect ratios; §2.3.3 prices.
        for (w, h) in [(5504, 3072), (6336, 2688), (5056, 3392), (4800, 3584), (4096, 4096)] {
            assert_eq!(google("gemini-3.1-flash-image", w, h), 151_000, "4K {w}x{h}");
        }
        for (w, h) in [(2752, 1536), (3168, 1344), (2528, 1696), (2400, 1792), (2048, 2048)] {
            assert_eq!(google("gemini-3.1-flash-image", w, h), 101_000, "2K {w}x{h}");
        }
        for (w, h) in [(1376, 768), (1584, 672), (1264, 848), (1200, 896), (1024, 1024)] {
            assert_eq!(google("gemini-3.1-flash-image", w, h), 67_000, "1K {w}x{h}");
        }
        assert_eq!(google("gemini-3.1-flash-image", 512, 512), 45_000);
    }

    #[test]
    fn other_gemini_image_models() {
        assert_eq!(google("gemini-3-pro-image", 5504, 3072), 240_000);
        assert_eq!(google("gemini-3-pro-image", 2752, 1536), 134_000);
        assert_eq!(google("gemini-3-pro-image", 1376, 768), 134_000);
        assert_eq!(google("gemini-3.1-flash-lite-image", 1376, 768), 33_600);
        assert_eq!(google("models/gemini-3-pro-image", 5504, 3072), 240_000);
        assert_eq!(google("gemini-3.1-flash-image-preview", 5504, 3072), 151_000);
        // Unknown (and the shut-down Nano Banana 1) → the default, Nano Banana 2.
        assert_eq!(google("gemini-2.5-flash-image", 5504, 3072), 151_000);
        assert_eq!(google("", 2752, 1536), 101_000);
    }

    #[test]
    fn local_providers_and_demo_are_free() {
        for kind in [ProviderKind::Ollama, ProviderKind::OpenAiCompatible, ProviderKind::ComfyUi, ProviderKind::Demo] {
            assert_eq!(image_cost(kind, "anything", 3840, 2160, HIGH), 0, "{kind:?}");
            assert_eq!(text_cost(kind, "anything", &TYPICAL_COMPOSE_USAGE), 0, "{kind:?}");
        }
    }

    #[test]
    fn zero_area_costs_nothing() {
        assert_eq!(openai("", 0, 2160, HIGH), 0);
        assert_eq!(google("", 3840, 0), 0);
    }

    #[test]
    fn text_costs_for_the_default_models() {
        // gpt-6-luna $0.10 / $0.50 per 1M: 1800 × 0.1 + 1600 × 0.5 = 980 µ$.
        assert_eq!(text_cost(ProviderKind::OpenAi, "gpt-6-luna", &TYPICAL_COMPOSE_USAGE), 980);
        // gemini-3.5-flash-lite $0.30 / $2.50: 540 + 4000.
        assert_eq!(text_cost(ProviderKind::Google, "gemini-3.5-flash-lite", &TYPICAL_COMPOSE_USAGE), 4540);
        let million = Usage { input_tokens: 1_000_000, output_tokens: 1_000_000 };
        assert_eq!(text_cost(ProviderKind::OpenAi, "gpt-6.1-sol", &million), 12_000_000);
        assert_eq!(text_cost(ProviderKind::OpenAi, "gpt-5-nano-2025-08-07", &million), 450_000);
        assert_eq!(text_cost(ProviderKind::Google, "gemini-3.1-flash-lite", &million), 1_750_000);
        assert_eq!(text_cost(ProviderKind::Google, "gemini-3.8-flash", &million), 9_000_000);
    }

    #[test]
    fn text_cost_rounds_up_and_handles_extremes() {
        let one = Usage { input_tokens: 1, output_tokens: 0 };
        assert_eq!(text_cost(ProviderKind::OpenAi, "gpt-6-luna", &one), 1);
        assert_eq!(text_cost(ProviderKind::OpenAi, "gpt-6-luna", &Usage::default()), 0);
        let huge = Usage { input_tokens: u64::MAX, output_tokens: u64::MAX };
        assert_eq!(text_cost(ProviderKind::OpenAi, "gpt-6-astra", &huge), u64::MAX);
    }

    #[test]
    fn unknown_text_models_price_as_the_provider_default() {
        let luna = text_cost(ProviderKind::OpenAi, openai::DEFAULT_TEXT_MODEL, &TYPICAL_COMPOSE_USAGE);
        assert_eq!(text_cost(ProviderKind::OpenAi, "gpt-7-nova", &TYPICAL_COMPOSE_USAGE), luna);
        assert_eq!(text_cost(ProviderKind::OpenAi, "", &TYPICAL_COMPOSE_USAGE), luna);
        let lite = text_cost(ProviderKind::Google, google::DEFAULT_TEXT_MODEL, &TYPICAL_COMPOSE_USAGE);
        // An image model's ID that starts like a text model's is not that text model.
        assert_eq!(text_cost(ProviderKind::Google, "gemini-3.1-flash-lite-image", &TYPICAL_COMPOSE_USAGE), lite);
    }

    #[test]
    fn model_matching() {
        assert!(is_model("gpt-image-2", "gpt-image-2"));
        assert!(is_model("gpt-image-2-2026-04-21", "gpt-image-2"));
        assert!(!is_model("gpt-image-2.5-flare", "gpt-image-2"));
        assert!(!is_model("gpt-image-1-mini", "gpt-image-1"));
        assert!(!is_model("gpt-5.4-nano", "gpt-5-nano"));
        assert!(is_model("gemini-3.5-flash-lite-001", "gemini-3.5-flash-lite"));
        assert!(is_model("gemini-3-pro-image-latest", "gemini-3-pro-image"));
        assert!(!is_model("gemini-3.1-flash-lite-image", "gemini-3.1-flash-lite"));
        assert_eq!(model_id(" Models/Gemini-3-Pro-Image "), "gemini-3-pro-image");
    }
}
