//! Taste: what the person tends to like, learned from Like/Dislike. Soft preferences only — Avoid
//! keywords are the hard rules. Counts decay by half per year so taste can drift.

use std::collections::{HashMap, HashSet};

use crate::model::{Concept, Rating, TasteSummary};
use crate::text::feature_key;

/// One learned feature (a normalised phrase from a concept; see `text::feature_key`).
#[derive(Debug, Clone, PartialEq)]
pub struct TasteRow {
    pub feature: String,
    /// Decayed counts as of `updated_at`.
    pub likes: f64,
    pub dislikes: f64,
    pub updated_at: i64,
}

impl TasteRow {
    /// The counts decayed from `updated_at` to `now`. A `now` before `updated_at` (a clock set back)
    /// leaves them as they are: counts only ever fade, never grow.
    fn evidence(&self, now: i64) -> Evidence {
        let elapsed_days = now.saturating_sub(self.updated_at).max(0) as f64 / SECONDS_PER_DAY;
        let factor = 0.5_f64.powf(elapsed_days / HALF_LIFE_DAYS);
        Evidence { likes: self.likes * factor, dislikes: self.dislikes * factor }
    }
}

/// Half-life of a rating's influence.
pub const HALF_LIFE_DAYS: f64 = 365.0;

const SECONDS_PER_DAY: f64 = 86_400.0;
/// Hints need at least this much (decayed) evidence: n = likes + dislikes.
const HINT_MIN_EVIDENCE: f64 = 2.0;
const HINT_LIKED_MEAN: f64 = 0.65;
const HINT_DISLIKED_MEAN: f64 = 0.35;
const HINTS_PER_SIDE: usize = 6;

/// Decayed counts for one feature at one moment.
#[derive(Debug, Clone, Copy)]
struct Evidence {
    likes: f64,
    dislikes: f64,
}

impl Evidence {
    fn n(self) -> f64 {
        self.likes + self.dislikes
    }

    /// Laplace-smoothed share of likes: (likes + 1) / (n + 2), 0.5 with no evidence.
    fn mean(self) -> f64 {
        (self.likes + 1.0) / (self.n() + 2.0)
    }

    /// `(mean − 0.5) × min(1, n/3)`: lean, damped until there are about three ratings.
    fn lean(self) -> f64 {
        (self.mean() - 0.5) * (self.n() / 3.0).min(1.0)
    }
}

/// Features of a concept: `feature_key` of setting, subject, each element, time_of_day, weather, season,
/// each mood, each palette colour, style, and each keyword used — deduplicated, empties dropped. In that
/// order (first occurrence kept).
pub fn features(concept: &Concept) -> Vec<String> {
    labelled_features(concept).into_iter().map(|(key, _)| key).collect()
}

/// `features` with the phrase each key came from (trimmed, whitespace collapsed, a leading article
/// dropped), for showing learned taste in words ("warm ivory", not the key "warm ivor"). Same order.
pub fn labelled_features(concept: &Concept) -> Vec<(String, String)> {
    let phrases = [&concept.setting, &concept.subject]
        .into_iter()
        .chain(&concept.elements)
        .chain([&concept.time_of_day, &concept.weather, &concept.season])
        .chain(&concept.mood)
        .chain(&concept.palette)
        .chain([&concept.style])
        .chain(&concept.keywords_used);
    let mut seen = HashSet::new();
    phrases
        .map(|phrase| (feature_key(phrase), label(phrase)))
        .filter(|(key, _)| !key.is_empty() && seen.insert(key.clone()))
        .collect()
}

fn label(phrase: &str) -> String {
    let words: Vec<&str> = phrase.split_whitespace().collect();
    let skip = usize::from(words.len() > 1 && matches!(words[0].to_lowercase().as_str(), "a" | "an" | "the"));
    words[skip..].join(" ")
}

/// Learned taste: decayed like and dislike counts per feature, kept in memory and saved row by row.
#[derive(Debug, Clone, Default)]
pub struct Taste {
    rows: HashMap<String, TasteRow>,
}

impl Taste {
    /// Builds taste from stored rows; a later row for the same feature replaces an earlier one.
    pub fn from_rows(rows: Vec<TasteRow>) -> Self {
        Self { rows: rows.into_iter().map(|row| (row.feature.clone(), row)).collect() }
    }

    /// Applies a rating change for a concept's features and returns the rows that changed (to save).
    /// `previous → current`: Unrated→Liked adds a like; Liked→Disliked removes the like (never below 0)
    /// and adds a dislike; →Unrated removes what the previous rating added; same → no change.
    /// Counts are decayed to `now` before changing.
    ///
    /// Repeated and empty features are ignored. Rows come back in `features` order; a feature that was
    /// never seen and would end at zero (removing a rating taste no longer knows) is not created.
    pub fn record(&mut self, features: &[String], previous: Rating, current: Rating, now: i64) -> Vec<TasteRow> {
        if previous == current {
            return Vec::new();
        }
        let (undo_likes, undo_dislikes) = counts_for(previous);
        let (add_likes, add_dislikes) = counts_for(current);

        let mut seen = HashSet::new();
        let mut changed = Vec::new();
        for feature in features {
            if feature.is_empty() || !seen.insert(feature.as_str()) {
                continue;
            }
            let existing = self.rows.get(feature);
            let before = existing.map_or(Evidence { likes: 0.0, dislikes: 0.0 }, |row| row.evidence(now));
            let likes = (before.likes - undo_likes).max(0.0) + add_likes;
            let dislikes = (before.dislikes - undo_dislikes).max(0.0) + add_dislikes;
            if existing.is_none() && likes == 0.0 && dislikes == 0.0 {
                continue;
            }
            let row = TasteRow {
                feature: feature.clone(),
                likes,
                dislikes,
                // Counts are as of the later of the two (see `TasteRow::evidence` on clocks set back).
                updated_at: existing.map_or(now, |row| row.updated_at.max(now)),
            };
            self.rows.insert(feature.clone(), row.clone());
            changed.push(row);
        }
        changed
    }

    /// Mean over the concept's known features of `((likes+1)/(n+2) − 0.5) × min(1, n/3)` (decayed to
    /// `now`, n = likes + dislikes); 0 when none are known. Range about −0.5..0.5. A feature is known when
    /// it has some evidence left (n > 0).
    pub fn score(&self, concept: &Concept, now: i64) -> f32 {
        let leans: Vec<f64> = features(concept)
            .iter()
            .filter_map(|feature| self.rows.get(feature))
            .map(|row| row.evidence(now))
            .filter(|evidence| evidence.n() > 0.0)
            .map(Evidence::lean)
            .collect();
        if leans.is_empty() {
            return 0.0;
        }
        (leans.iter().sum::<f64>() / leans.len() as f64) as f32
    }

    /// (liked, disliked): up to 6 each, strongest evidence first; liked = mean ≥ 0.65 with n ≥ 2,
    /// disliked = mean ≤ 0.35 with n ≥ 2 (mean = (likes+1)/(n+2), decayed to `now`).
    ///
    /// Strength is the size of the feature's lean in `score` (`|mean − 0.5| × min(1, n/3)`); ties go to
    /// more evidence, then alphabetical order. Because counts decay continuously, exactly two ratings fall
    /// just under n = 2 as soon as any time passes, so in practice a hint takes three ratings (which stay
    /// above 2 for about seven months).
    pub fn hints(&self, now: i64) -> (Vec<String>, Vec<String>) {
        let mut liked = Vec::new();
        let mut disliked = Vec::new();
        for row in self.rows.values() {
            let evidence = row.evidence(now);
            if evidence.n() < HINT_MIN_EVIDENCE {
                continue;
            }
            let mean = evidence.mean();
            if mean >= HINT_LIKED_MEAN {
                liked.push((evidence, row.feature.as_str()));
            } else if mean <= HINT_DISLIKED_MEAN {
                disliked.push((evidence, row.feature.as_str()));
            }
        }
        (strongest(liked), strongest(disliked))
    }

    pub fn summary(&self, now: i64, ratings: u32) -> TasteSummary {
        let (liked, disliked) = self.hints(now);
        TasteSummary { liked, disliked, ratings }
    }
}

/// (likes, dislikes) a rating contributes.
fn counts_for(rating: Rating) -> (f64, f64) {
    match rating {
        Rating::Liked => (1.0, 0.0),
        Rating::Disliked => (0.0, 1.0),
        Rating::Unrated => (0.0, 0.0),
    }
}

/// Up to `HINTS_PER_SIDE` features, strongest first (see `Taste::hints`).
fn strongest(mut features: Vec<(Evidence, &str)>) -> Vec<String> {
    // Strength is compared to 9 decimal places, so leans that are equal on paper (2 likes vs 3 likes and a
    // dislike) tie instead of being ordered by floating-point noise. Integer keys keep the order total.
    let strength = |evidence: &Evidence| (evidence.lean().abs() * 1e9).round() as i64;
    features.sort_by(|(a, a_feature), (b, b_feature)| {
        strength(b)
            .cmp(&strength(a))
            .then_with(|| b.n().total_cmp(&a.n()))
            .then_with(|| a_feature.cmp(b_feature))
    });
    features.into_iter().take(HINTS_PER_SIDE).map(|(_, feature)| feature.to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_keep_the_words_the_concept_used() {
        let concept = Concept {
            setting: "The terraced garden".into(),
            palette: vec!["warm  ivory".into(), "Warm ivory".into()],
            keywords_used: vec!["lanterns".into()],
            ..Concept::default()
        };
        assert_eq!(
            labelled_features(&concept),
            [
                ("terrac garden".to_string(), "terraced garden".to_string()),
                ("warm ivor".to_string(), "warm ivory".to_string()),
                ("lantern".to_string(), "lanterns".to_string()),
            ]
        );
        assert_eq!(features(&concept), ["terrac garden", "warm ivor", "lantern"]);
    }

    const DAY: i64 = 86_400;
    const YEAR: i64 = 365 * DAY;
    const NOW: i64 = 1_800_000_000;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    fn keys(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn concept() -> Concept {
        Concept {
            title: "Rainy ruins".into(),
            summary: "Old ruins in the rain.".into(),
            setting: "The Ancient Ruins".into(),
            subject: "a stone arch".into(),
            elements: vec!["puddles".into(), "Lanterns".into(), "".into(), "lantern".into()],
            time_of_day: "night".into(),
            weather: "rainy".into(),
            season: "".into(),
            mood: vec!["peaceful".into()],
            palette: vec!["deep blue".into(), "amber".into()],
            style: "Matte painting".into(),
            composition: "low angle".into(),
            keywords_used: vec!["rain".into(), "ruins".into()],
            wildcards: vec!["owl".into()],
            prompt: "…".into(),
        }
    }

    fn row(feature: &str, likes: f64, dislikes: f64, updated_at: i64) -> TasteRow {
        TasteRow { feature: feature.into(), likes, dislikes, updated_at }
    }

    #[test]
    fn features_are_normalised_deduplicated_and_ordered() {
        assert_eq!(
            features(&concept()),
            keys(&[
                "ancient ruin", "stone arch", "puddle", "lantern", "night", "rain", "peaceful", "deep blue", "amber",
                "matte paint", "ruin",
            ])
        );
        assert!(features(&Concept::default()).is_empty());
    }

    #[test]
    fn like_then_dislike_then_unrate() {
        let mut taste = Taste::default();
        let f = keys(&["rain", "ruin"]);

        let changed = taste.record(&f, Rating::Unrated, Rating::Liked, NOW);
        assert_eq!(changed, vec![row("rain", 1.0, 0.0, NOW), row("ruin", 1.0, 0.0, NOW)]);

        let changed = taste.record(&f, Rating::Liked, Rating::Disliked, NOW);
        assert_eq!(changed, vec![row("rain", 0.0, 1.0, NOW), row("ruin", 0.0, 1.0, NOW)]);

        let changed = taste.record(&f, Rating::Disliked, Rating::Unrated, NOW);
        assert_eq!(changed, vec![row("rain", 0.0, 0.0, NOW), row("ruin", 0.0, 0.0, NOW)]);

        let changed = taste.record(&f, Rating::Unrated, Rating::Disliked, NOW);
        assert_eq!(changed[0], row("rain", 0.0, 1.0, NOW));
        let changed = taste.record(&f, Rating::Disliked, Rating::Liked, NOW);
        assert_eq!(changed[0], row("rain", 1.0, 0.0, NOW));
    }

    #[test]
    fn same_rating_changes_nothing() {
        let mut taste = Taste::default();
        assert!(taste.record(&keys(&["rain"]), Rating::Liked, Rating::Liked, NOW).is_empty());
        assert!(taste.record(&keys(&["rain"]), Rating::Unrated, Rating::Unrated, NOW).is_empty());
        assert_eq!(taste.score(&concept(), NOW), 0.0);
    }

    #[test]
    fn removing_never_goes_below_zero_and_unknown_features_are_not_created() {
        let mut taste = Taste::from_rows(vec![row("rain", 0.4, 0.0, NOW)]);
        let changed = taste.record(&keys(&["rain", "fog"]), Rating::Liked, Rating::Disliked, NOW);
        assert_eq!(changed, vec![row("rain", 0.0, 1.0, NOW), row("fog", 0.0, 1.0, NOW)]);

        // Liked → Unrated for features taste never saw (after a reset): nothing to save.
        let changed = taste.record(&keys(&["mist"]), Rating::Liked, Rating::Unrated, NOW);
        assert!(changed.is_empty());
    }

    #[test]
    fn repeated_and_empty_features_count_once() {
        let mut taste = Taste::default();
        let changed = taste.record(&keys(&["rain", "", "rain"]), Rating::Unrated, Rating::Liked, NOW);
        assert_eq!(changed, vec![row("rain", 1.0, 0.0, NOW)]);
    }

    #[test]
    fn counts_decay_by_half_per_year_before_changing() {
        let mut taste = Taste::from_rows(vec![row("rain", 4.0, 2.0, NOW - YEAR)]);
        let changed = taste.record(&keys(&["rain"]), Rating::Unrated, Rating::Liked, NOW);
        assert_eq!(changed.len(), 1);
        assert!(close(changed[0].likes, 3.0));
        assert!(close(changed[0].dislikes, 1.0));
        assert_eq!(changed[0].updated_at, NOW);

        // Liked → Disliked a year later removes a (decayed) like: 1.5 − 1.
        let changed = taste.record(&keys(&["rain"]), Rating::Liked, Rating::Disliked, NOW + YEAR);
        assert!(close(changed[0].likes, 0.5));
        assert!(close(changed[0].dislikes, 1.5));
    }

    #[test]
    fn a_clock_set_back_never_grows_counts() {
        let taste = Taste::from_rows(vec![row("rain", 2.0, 0.0, NOW)]);
        let earlier = taste.rows["rain"].evidence(NOW - YEAR);
        assert!(close(earlier.likes, 2.0));

        let mut taste = taste;
        let changed = taste.record(&keys(&["rain"]), Rating::Unrated, Rating::Liked, NOW - DAY);
        assert_eq!(changed, vec![row("rain", 3.0, 0.0, NOW)]);
    }

    #[test]
    fn score_is_mean_lean_over_known_features() {
        // rain: 3 likes → mean 4/5 = 0.8, lean 0.3 × 1. night: 1 dislike → mean 1/3, lean −1/6 × 1/3.
        // "puddle" has no evidence left and doesn't count; other features are unknown.
        let taste = Taste::from_rows(vec![
            row("rain", 3.0, 0.0, NOW),
            row("night", 0.0, 1.0, NOW),
            row("puddle", 0.0, 0.0, NOW),
            row("desert", 5.0, 0.0, NOW),
        ]);
        let expected = (0.3 + (1.0 / 3.0 - 0.5) / 3.0) / 2.0;
        assert!((f64::from(taste.score(&concept(), NOW)) - expected).abs() < 1e-6);
    }

    #[test]
    fn score_decays_toward_zero() {
        let taste = Taste::from_rows(vec![row("rain", 3.0, 0.0, NOW)]);
        let fresh = taste.score(&concept(), NOW);
        let later = taste.score(&concept(), NOW + 3 * YEAR);
        assert!(fresh > later && later > 0.0, "{fresh} {later}");
        assert_eq!(Taste::default().score(&concept(), NOW), 0.0);
    }

    #[test]
    fn score_stays_within_half() {
        let loved = Taste::from_rows(features(&concept()).iter().map(|f| row(f, 1e6, 0.0, NOW)).collect());
        let hated = Taste::from_rows(features(&concept()).iter().map(|f| row(f, 0.0, 1e6, NOW)).collect());
        let high = loved.score(&concept(), NOW);
        let low = hated.score(&concept(), NOW);
        assert!(high > 0.49 && high < 0.5, "{high}");
        assert!(low < -0.49 && low > -0.5, "{low}");
    }

    #[test]
    fn hints_need_evidence_and_a_clear_lean() {
        let taste = Taste::from_rows(vec![
            row("rain", 2.0, 0.0, NOW),        // mean .75, n 2 → liked
            row("fog", 3.0, 1.0, NOW),         // mean .667, n 4 → liked
            row("neon", 4.0, 2.0, NOW),        // mean .625 → neither
            row("desert", 0.0, 3.0, NOW),      // mean .2 → disliked
            row("crowd", 1.0, 3.0, NOW),       // mean .333 → disliked
            row("moon", 1.0, 0.0, NOW),        // n 1 → too little evidence
            row("sand", 0.0, 1.9, NOW),        // n 1.9 → too little evidence
        ]);
        let (liked, disliked) = taste.hints(NOW);
        // rain: |.25| × 2/3 = .1667; fog: |.1667| × 1 = .1667 → tie on strength, fog has more evidence.
        assert_eq!(liked, keys(&["fog", "rain"]));
        assert_eq!(disliked, keys(&["desert", "crowd"]));
    }

    #[test]
    fn hints_keep_six_per_side_with_a_deterministic_order() {
        let rows: Vec<TasteRow> = ["h", "g", "f", "e", "d", "c", "b", "a"]
            .iter()
            .map(|f| row(f, 5.0, 0.0, NOW))
            .chain([row("strong", 20.0, 0.0, NOW)])
            .collect();
        let (liked, disliked) = Taste::from_rows(rows).hints(NOW);
        assert_eq!(liked, keys(&["strong", "a", "b", "c", "d", "e"]));
        assert!(disliked.is_empty());
    }

    #[test]
    fn hints_fade_with_time() {
        let taste = Taste::from_rows(vec![row("rain", 3.0, 0.0, NOW)]);
        assert_eq!(taste.hints(NOW).0, keys(&["rain"]));
        // A year on, 1.5 likes are left: under n = 2.
        assert!(taste.hints(NOW + YEAR).0.is_empty());
    }

    #[test]
    fn summary_wraps_hints() {
        let taste = Taste::from_rows(vec![row("rain", 3.0, 0.0, NOW), row("desert", 0.0, 3.0, NOW)]);
        assert_eq!(
            taste.summary(NOW, 12),
            TasteSummary { liked: keys(&["rain"]), disliked: keys(&["desert"]), ratings: 12 }
        );
    }

    #[test]
    fn later_rows_replace_earlier_ones() {
        let taste = Taste::from_rows(vec![row("rain", 9.0, 0.0, NOW), row("rain", 0.0, 3.0, NOW)]);
        assert_eq!(taste.hints(NOW), (vec![], keys(&["rain"])));
    }
}
