//! Text helpers shared by keywords, the composer's checks and taste features.
//!
//! Matching is deliberately simple and predictable: lower-case, Unicode-aware word splitting, and a light
//! English stemmer (plurals and common suffixes), so "rain" matches "rainy"/"raining", "ruins" matches
//! "ruin", "lanterns" matches "lantern", "foggy" matches "fog". Words in other languages pass through
//! mostly unchanged (exact word match).

use crate::error::{AutoPaperError, InvalidInputReason, Result};
use crate::model::{Keyword, KeywordWeight, Mood};

/// Trims, collapses inner whitespace to single spaces, strips control characters; rejects empty text,
/// text over `Keyword::MAX_LEN` characters (after normalising), and text with no letters or digits.
pub fn normalize_keyword(text: &str) -> Result<String> {
    let cleaned: String = text.chars().filter(|c| !c.is_control()).collect();
    let normalized = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    if !normalized.chars().any(char::is_alphanumeric) {
        return Err(AutoPaperError::invalid_input(
            InvalidInputReason::KeywordEmpty,
            "a keyword needs at least one letter or digit",
        ));
    }
    if normalized.chars().count() > Keyword::MAX_LEN {
        return Err(AutoPaperError::invalid_input(
            InvalidInputReason::KeywordTooLong,
            format!("keywords can be up to {} characters", Keyword::MAX_LEN),
        ));
    }
    Ok(normalized)
}

/// A mood's name as stored: trimmed, inner whitespace collapsed to single spaces, control characters stripped.
/// `InvalidInput` `MoodNameEmpty` when nothing is left, `MoodNameTooLong` past `Mood::MAX_NAME_LEN` characters.
pub fn normalize_mood_name(name: &str) -> Result<String> {
    let cleaned: String = name.chars().filter(|c| !c.is_control()).collect();
    let normalized = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        return Err(AutoPaperError::invalid_input(InvalidInputReason::MoodNameEmpty, "a mood needs a name"));
    }
    if normalized.chars().count() > Mood::MAX_NAME_LEN {
        return Err(AutoPaperError::invalid_input(
            InvalidInputReason::MoodNameTooLong,
            format!("mood names can be up to {} characters", Mood::MAX_NAME_LEN),
        ));
    }
    Ok(normalized)
}

/// The name the first mood gets from the keywords it was made of (the store's migration to moods): its first two
/// keywords that aren't Avoids, each with a capital first letter, "Rain, Beach" (just the first when both don't
/// fit `Mood::MAX_NAME_LEN`), else "My mood". An Avoid names what's left out, so it never names the mood.
pub fn mood_name_from_keywords<'a>(keywords: impl IntoIterator<Item = (&'a str, KeywordWeight)>) -> String {
    let named: Vec<String> = keywords
        .into_iter()
        .filter(|(_, weight)| *weight != KeywordWeight::Avoid)
        .filter_map(|(text, _)| normalize_mood_name(text).ok())
        .take(2)
        .map(|text| capitalised(&text))
        .collect();
    match named.as_slice() {
        [] => DEFAULT_MOOD_NAME.to_string(),
        [first, ..] => {
            let both = named.join(", ");
            if both.chars().count() <= Mood::MAX_NAME_LEN { both } else { first.clone() }
        }
    }
}

/// What a mood is called when nothing better names it.
pub const DEFAULT_MOOD_NAME: &str = "My mood";

/// `text` with its first character upper-cased (Unicode-aware; "über" → "Über").
fn capitalised(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Lower-cased words of `text` (letters and digits; an apostrophe inside a word is kept), in order.
pub fn words(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut current = String::new();
    for (i, &c) in chars.iter().enumerate() {
        let inner_apostrophe = (c == '\'' || c == '’')
            && !current.is_empty()
            && chars.get(i + 1).is_some_and(|next| next.is_alphanumeric());
        if c.is_alphanumeric() || inner_apostrophe {
            current.extend(c.to_lowercase());
        } else if !current.is_empty() {
            out.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// A light English stem. Lower-cases, strips a possessive "'s", then applies the first matching rule:
/// - "ies" → "y" (stories → story)
/// - "ing" / "ed" when at least 4 letters remain (raining → rain, weathered → weather; spring, red stay)
/// - "es" after s/x/z/ch/sh (beaches → beach, glasses → glass)
/// - "s", except after s/u/i (ruins → ruin; glass, chaos-like "us"/"is" words stay)
/// - "y" when at least 4 letters remain (rainy → rain, stormy → storm; sky stays)
///
/// After "ing"/"ed"/"y", a doubled final consonant other than l/s/z is undoubled (foggy → fog,
/// running → run; falling → fall). Results always keep at least 3 letters, else the word is unchanged.
pub fn stem(word: &str) -> String {
    let mut w = word.to_lowercase();
    for possessive in ["'s", "’s"] {
        if let Some(stripped) = w.strip_suffix(possessive) {
            w = stripped.to_string();
        }
    }
    let letters = |s: &str| s.chars().count();

    if let Some(base) = w.strip_suffix("ies")
        && letters(base) >= 2
    {
        return format!("{base}y");
    }
    for suffix in ["ing", "ed"] {
        if let Some(base) = w.strip_suffix(suffix)
            && letters(base) >= 4
        {
            return undouble(base);
        }
    }
    if let Some(base) = w.strip_suffix("es")
        && ["s", "x", "z", "ch", "sh"].iter().any(|end| base.ends_with(end))
        && letters(base) >= 3
    {
        return base.to_string();
    }
    if let Some(base) = w.strip_suffix('s')
        && !base.ends_with(['s', 'u', 'i'])
        && letters(base) >= 3
    {
        return base.to_string();
    }
    if let Some(base) = w.strip_suffix('y')
        && letters(base) >= 4
    {
        return undouble(base);
    }
    w
}

/// "fogg" → "fog", "runn" → "run"; keeps ll/ss/zz and anything that would drop below 3 letters.
fn undouble(base: &str) -> String {
    let chars: Vec<char> = base.chars().collect();
    let n = chars.len();
    if n >= 4 && chars[n - 1] == chars[n - 2] && !matches!(chars[n - 1], 'l' | 's' | 'z') && !is_vowel(chars[n - 1]) {
        return chars[..n - 1].iter().collect();
    }
    base.to_string()
}

fn is_vowel(c: char) -> bool {
    matches!(c, 'a' | 'e' | 'i' | 'o' | 'u')
}

/// True when every word of `term` appears in `haystack` as consecutive words, comparing stems
/// ("blue hour" matches "the blue hours", "rain" matches "rainy night"; "art" does not match "party").
/// An empty term never matches.
pub fn mentions(haystack: &str, term: &str) -> bool {
    let needle: Vec<String> = words(term).iter().map(|w| stem(w)).collect();
    if needle.is_empty() {
        return false;
    }
    let hay: Vec<String> = words(haystack).iter().map(|w| stem(w)).collect();
    hay.windows(needle.len()).any(|window| window == needle.as_slice())
}

/// A normalised key for a taste feature: stems of the phrase's words joined by single spaces, with a
/// leading article ("a", "an", "the") dropped. Empty input → empty key (callers skip it).
pub fn feature_key(phrase: &str) -> String {
    let mut ws = words(phrase);
    if ws.len() > 1 && matches!(ws[0].as_str(), "a" | "an" | "the") {
        ws.remove(0);
    }
    ws.iter().map(|w| stem(w)).collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_mood_names() {
        assert_eq!(normalize_mood_name("  Rainy \t beach \n").unwrap(), "Rainy beach");
        assert_eq!(normalize_mood_name("☔️").unwrap(), "☔️", "any visible text names a mood");
        for (name, why) in [("   ", InvalidInputReason::MoodNameEmpty), ("\u{7}", InvalidInputReason::MoodNameEmpty)] {
            assert!(matches!(normalize_mood_name(name), Err(AutoPaperError::InvalidInput { reason, .. }) if reason == why), "{name:?}");
        }
        assert_eq!(normalize_mood_name(&"é".repeat(Mood::MAX_NAME_LEN)).unwrap().chars().count(), Mood::MAX_NAME_LEN);
        assert!(matches!(
            normalize_mood_name(&"x".repeat(Mood::MAX_NAME_LEN + 1)),
            Err(AutoPaperError::InvalidInput { reason: InvalidInputReason::MoodNameTooLong, .. })
        ));
    }

    #[test]
    fn the_first_mood_is_named_after_its_first_two_keywords() {
        use KeywordWeight::{Avoid, Maybe, Must};
        assert_eq!(mood_name_from_keywords([("rain", Must), ("beach", Maybe), ("night", Must)]), "Rain, Beach");
        assert_eq!(mood_name_from_keywords([("people", Avoid), ("über alles", Must)]), "Über alles", "Avoids never name it");
        assert_eq!(mood_name_from_keywords([("people", Avoid)]), "My mood");
        assert_eq!(mood_name_from_keywords(std::iter::empty::<(&str, KeywordWeight)>()), "My mood");
        let long = "a".repeat(30);
        assert_eq!(mood_name_from_keywords([(long.as_str(), Must), ("lighthouse", Must)]), format!("A{}", "a".repeat(29)));
    }

    #[test]
    fn normalizes_keywords() {
        assert_eq!(normalize_keyword("  blue \t hour \n").unwrap(), "blue hour");
        assert_eq!(normalize_keyword("Rosé").unwrap(), "Rosé");
        let reason = |text: &str| match normalize_keyword(text) {
            Err(AutoPaperError::InvalidInput { reason, .. }) => reason,
            other => panic!("{text:?}: {other:?}"),
        };
        assert_eq!(reason("   "), InvalidInputReason::KeywordEmpty);
        assert_eq!(reason("•••"), InvalidInputReason::KeywordEmpty);
        assert_eq!(reason(&"a".repeat(41)), InvalidInputReason::KeywordTooLong);
        assert_eq!(normalize_keyword(&"é".repeat(40)).unwrap().chars().count(), 40);
        assert_eq!(normalize_keyword("rain\u{0007}drops").unwrap(), "raindrops");
    }

    #[test]
    fn splits_words() {
        assert_eq!(words("Rain • ruins, peaceful-night"), ["rain", "ruins", "peaceful", "night"]);
        assert_eq!(words("the sea's edge 'quoted'"), ["the", "sea's", "edge", "quoted"]);
        assert_eq!(words("Øresund at 6am"), ["øresund", "at", "6am"]);
    }

    #[test]
    fn stems() {
        let cases = [
            ("rain", "rain"), ("rainy", "rain"), ("raining", "rain"), ("rained", "rain"),
            ("ruins", "ruin"), ("ruin", "ruin"), ("lanterns", "lantern"),
            ("foggy", "fog"), ("fog", "fog"), ("stormy", "storm"), ("cloudy", "cloud"),
            ("stories", "story"), ("beaches", "beach"), ("glasses", "glass"), ("glass", "glass"),
            ("spring", "spring"), ("springs", "spring"), ("red", "red"), ("sky", "sky"),
            ("weathered", "weather"), ("falling", "fall"), ("running", "run"), ("moss", "moss"),
            ("city's", "city"), ("art", "art"), ("party", "part"),
        ];
        for (word, expected) in cases {
            assert_eq!(stem(word), expected, "stem({word})");
        }
    }

    #[test]
    fn mentions_by_stem_and_phrase() {
        let prompt = "Ancient ruins in the rainy blue hours, a peaceful night";
        assert!(mentions(prompt, "rain"));
        assert!(mentions(prompt, "ruins"));
        assert!(mentions(prompt, "Ruin"));
        assert!(mentions(prompt, "blue hour"));
        assert!(mentions(prompt, "night"));
        assert!(!mentions(prompt, "hour blue"));
        assert!(!mentions(prompt, "neon"));
        assert!(!mentions("a party at dusk", "art"));
        assert!(!mentions(prompt, "  "));
    }

    #[test]
    fn feature_keys() {
        assert_eq!(feature_key("The Blue Hours"), "blue hour");
        assert_eq!(feature_key("a"), "a");
        assert_eq!(feature_key("Misty Mountains"), "mist mountain");
        assert_eq!(feature_key(""), "");
    }
}
