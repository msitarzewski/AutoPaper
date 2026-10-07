//! Echoes: an old idea, returned as a new interpretation once its quiet period has passed.

use std::collections::HashSet;

use rand::Rng;

use crate::model::{Concept, Rating};
use crate::novelty::LineageIndex;
use crate::store::MemoryRow;

const SECONDS_PER_DAY: i64 = 86_400;
const DAYS_PER_YEAR: f64 = 365.0;

/// What changes between an original and its echo. The composer turns each into an instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EchoAxis {
    Weather,
    TimeOfDay,
    Season,
    /// Decay or renewal: weathered, overgrown, restored, rebuilt.
    PassageOfTime,
    /// Closer, further, higher, from another side.
    Viewpoint,
    /// Painting ↔ photograph ↔ illustration, or another era's look.
    Medium,
}

impl EchoAxis {
    pub const ALL: [EchoAxis; 6] = [
        EchoAxis::Weather,
        EchoAxis::TimeOfDay,
        EchoAxis::Season,
        EchoAxis::PassageOfTime,
        EchoAxis::Viewpoint,
        EchoAxis::Medium,
    ];

    /// The instruction given to the composer, in English.
    pub fn instruction(self) -> &'static str {
        match self {
            EchoAxis::Weather => "Change the weather (for example clear after rain, a storm passing, snow, haze).",
            EchoAxis::TimeOfDay => "Move it to a different time of day (for example dawn instead of night).",
            EchoAxis::Season => "Move it to a different season.",
            EchoAxis::PassageOfTime => {
                "Let years pass in the scene: weathered, overgrown or ruined — or restored and rebuilt."
            }
            EchoAxis::Viewpoint => "See it from a different viewpoint or distance (closer, further, higher, another side).",
            EchoAxis::Medium => "Render it in a different medium or era's look (painting, photograph, illustration).",
        }
    }
}

/// Originals that may be echoed now: status ok (memory holds only ok rows), not disliked, older than the
/// quiet period, and whose lineage (see `novelty::lineage_root`) has had no echo created within the quiet
/// period. Echoes themselves can be echoed (the lineage check keeps it rare).
///
/// "Older than" is strictly more than `quiet_days` (a negative quiet period counts as 0); an echo exactly
/// `quiet_days` old is still within it. Memory order is kept.
pub fn eligible(memory: &[MemoryRow], now: i64, quiet_days: i64) -> Vec<&MemoryRow> {
    let quiet_secs = quiet_days.max(0).saturating_mul(SECONDS_PER_DAY);
    let lineages = LineageIndex::new(memory);
    let recently_echoed: HashSet<&str> = memory
        .iter()
        .filter(|row| row.echo_of.is_some() && now.saturating_sub(row.created_at) <= quiet_secs)
        .map(|row| lineages.root(&row.id))
        .collect();
    memory
        .iter()
        .filter(|row| rating_allows(row.rating))
        .filter(|row| now.saturating_sub(row.created_at) > quiet_secs)
        .filter(|row| !recently_echoed.contains(lineages.root(&row.id)))
        .collect()
}

/// Picks one eligible original, weighted: liked ×3, unrated ×1; older ones a little more likely
/// (weight × (1 + years since created, capped at 3)); made under `mood` (the active mood) ×`MOOD_PREFERENCE`, so
/// echoes mostly return to what the person is in the mood for now, yet any mood's may come back. `None` when
/// `eligible` is empty (or holds only disliked rows, which weigh nothing).
pub fn pick<'a, R: Rng + ?Sized>(
    eligible: &[&'a MemoryRow],
    now: i64,
    mood: Option<&str>,
    rng: &mut R,
) -> Option<&'a MemoryRow> {
    let weights: Vec<f64> = eligible.iter().map(|row| pick_weight(row, now, mood)).collect();
    weighted_index(&weights, rng).and_then(|index| eligible.get(index).copied())
}

/// How much more likely an original made under the active mood is to be echoed.
pub const MOOD_PREFERENCE: f64 = 3.0;

fn pick_weight(row: &MemoryRow, now: i64, mood: Option<&str>) -> f64 {
    let rating = match row.rating {
        Rating::Liked => 3.0,
        Rating::Unrated => 1.0,
        Rating::Disliked => 0.0,
    };
    let years = now.saturating_sub(row.created_at).max(0) as f64 / (SECONDS_PER_DAY as f64 * DAYS_PER_YEAR);
    let in_mood = if mood.is_some() && row.mood_id.as_deref() == mood { MOOD_PREFERENCE } else { 1.0 };
    rating * (1.0 + years.min(3.0)) * in_mood
}

/// 2 axes at surprise < 0.5, 3 otherwise; distinct, random. An axis the original already pins down (it
/// has a time of day, a weather or a season) is twice as likely as the others — changing what's there
/// reads clearly as an echo — but any axis may be chosen.
pub fn choose_axes<R: Rng + ?Sized>(surprise: f32, original: &Concept, rng: &mut R) -> Vec<EchoAxis> {
    let count = if surprise < 0.5 { 2 } else { 3 };
    let mut pool: Vec<EchoAxis> = EchoAxis::ALL.to_vec();
    let mut chosen = Vec::with_capacity(count);
    while chosen.len() < count {
        let weights: Vec<f64> = pool.iter().map(|axis| axis_weight(*axis, original)).collect();
        let Some(index) = weighted_index(&weights, rng) else { break };
        chosen.push(pool.remove(index));
    }
    chosen
}

fn axis_weight(axis: EchoAxis, original: &Concept) -> f64 {
    let present = |field: &str| !field.trim().is_empty();
    let pinned = match axis {
        EchoAxis::TimeOfDay => present(&original.time_of_day),
        EchoAxis::Weather => present(&original.weather),
        EchoAxis::Season => present(&original.season),
        EchoAxis::PassageOfTime | EchoAxis::Viewpoint | EchoAxis::Medium => false,
    };
    if pinned { 2.0 } else { 1.0 }
}

/// Index drawn with probability proportional to its weight; `None` when no weight is positive.
fn weighted_index<R: Rng + ?Sized>(weights: &[f64], rng: &mut R) -> Option<usize> {
    let total: f64 = weights.iter().filter(|w| **w > 0.0).sum();
    if !(total > 0.0 && total.is_finite()) {
        return None;
    }
    let target = rng.random::<f64>() * total;
    let mut cumulative = 0.0;
    for (index, weight) in weights.iter().enumerate() {
        if *weight > 0.0 {
            cumulative += weight;
            if target < cumulative {
                return Some(index);
            }
        }
    }
    // Rounding can leave `target` a hair past the last sum.
    weights.iter().rposition(|w| *w > 0.0)
}

/// "3 weeks ago", "4 months ago", "about a year ago", "2 years ago" — plain English for the composer
/// (the model writes the echo note in the person's language).
///
/// Under a day it counts hours ("less than an hour ago", "5 hours ago"), then days, weeks (from 7 days),
/// months (from 30 days, 30.44 days each), "about a year ago" from 11.5 months to 1.5 years, then whole
/// years, rounded. Negative ages count as 0.
pub fn age_text(seconds: i64) -> String {
    const HOUR: i64 = 3_600;
    let seconds = seconds.max(0);
    let days = seconds as f64 / SECONDS_PER_DAY as f64;
    let ago = |n: i64, one: &str, unit: &str| if n == 1 { format!("{one} ago") } else { format!("{n} {unit} ago") };

    if seconds < HOUR {
        "less than an hour ago".to_string()
    } else if seconds < SECONDS_PER_DAY {
        ago(seconds / HOUR, "an hour", "hours")
    } else if days < 7.0 {
        ago(seconds / SECONDS_PER_DAY, "a day", "days")
    } else if days < 30.0 {
        ago((days / 7.0).round() as i64, "a week", "weeks")
    } else if (days / 30.44).round() < 12.0 {
        ago((days / 30.44).round() as i64, "a month", "months")
    } else if days / 365.25 < 1.5 {
        "about a year ago".to_string()
    } else {
        ago((days / 365.25).round() as i64, "a year", "years")
    }
}

/// Whether a rating allows echoing (anything but Disliked).
pub fn rating_allows(rating: Rating) -> bool {
    rating != Rating::Disliked
}

#[cfg(test)]
mod tests {
    use rand::rngs::StdRng;
    use rand::{RngCore, SeedableRng};

    use super::*;

    const DAY: i64 = 86_400;
    const NOW: i64 = 1_800_000_000;
    const Q: i64 = 182;

    fn row(id: &str, days_ago: i64, rating: Rating, echo_of: Option<&str>) -> MemoryRow {
        MemoryRow {
            id: id.into(),
            created_at: NOW - days_ago * DAY,
            title: format!("title {id}"),
            summary: format!("summary {id}"),
            embedding: vec![1.0],
            embedding_model: "test".into(),
            rating,
            echo_of: echo_of.map(Into::into),
            mood_id: Some("rain".into()),
        }
    }

    fn ids<'a>(rows: impl IntoIterator<Item = &'a MemoryRow>) -> Vec<&'a str> {
        rows.into_iter().map(|row| row.id.as_str()).collect()
    }

    /// Always returns the same word: 0 draws the bottom of [0, 1), `u64::MAX` the top.
    struct ConstRng(u64);

    impl RngCore for ConstRng {
        fn next_u32(&mut self) -> u32 {
            (self.0 >> 32) as u32
        }
        fn next_u64(&mut self) -> u64 {
            self.0
        }
        fn fill_bytes(&mut self, dst: &mut [u8]) {
            for chunk in dst.chunks_mut(8) {
                let bytes = self.0.to_le_bytes();
                chunk.copy_from_slice(&bytes[..chunk.len()]);
            }
        }
    }

    #[test]
    fn eligible_needs_age_a_rating_that_allows_it_and_a_quiet_lineage() {
        let memory = vec![
            row("liked-old", 400, Rating::Liked, None),
            row("unrated-old", 300, Rating::Unrated, None),
            row("disliked-old", 300, Rating::Disliked, None),
            row("recent", 10, Rating::Liked, None),
            // Echoed 20 days ago: the whole lineage rests.
            row("resting-root", 500, Rating::Liked, None),
            row("resting-echo", 20, Rating::Unrated, Some("resting-root")),
            // Echoed long ago: root and echo both eligible again.
            row("old-root", 900, Rating::Unrated, None),
            row("old-echo", 600, Rating::Liked, Some("old-root")),
            // A recent echo of an old echo still rests the whole lineage, root included.
            row("deep-root", 1000, Rating::Liked, None),
            row("deep-1", 700, Rating::Liked, Some("deep-root")),
            row("deep-2", 5, Rating::Liked, Some("deep-1")),
        ];
        assert_eq!(
            ids(eligible(&memory, NOW, Q)),
            ["liked-old", "unrated-old", "old-root", "old-echo"]
        );
    }

    #[test]
    fn eligible_boundaries() {
        let at_q = vec![row("a", Q, Rating::Liked, None)];
        assert!(eligible(&at_q, NOW, Q).is_empty());
        assert_eq!(ids(eligible(&at_q, NOW + 1, Q)), ["a"]);

        // An echo exactly Q old still rests its lineage; a second later it doesn't.
        let memory = vec![row("root", 3 * Q, Rating::Liked, None), row("echo", Q, Rating::Liked, Some("root"))];
        assert!(eligible(&memory, NOW, Q).is_empty());
        assert_eq!(ids(eligible(&memory, NOW + 1, Q)), ["root", "echo"]);
    }

    #[test]
    fn eligible_with_empty_memory_or_no_quiet_period() {
        assert!(eligible(&[], NOW, Q).is_empty());
        let memory = vec![row("a", 1, Rating::Unrated, None), row("b", 0, Rating::Unrated, None)];
        assert_eq!(ids(eligible(&memory, NOW, 0)), ["a"]);
        assert_eq!(ids(eligible(&memory, NOW, -5)), ["a"]);
    }

    #[test]
    fn eligible_echoes_of_a_forgotten_original_share_a_lineage() {
        let memory = vec![
            row("orphan-old", 400, Rating::Liked, Some("forgotten")),
            row("orphan-new", 3, Rating::Liked, Some("forgotten")),
        ];
        assert!(eligible(&memory, NOW, Q).is_empty());
    }

    #[test]
    fn pick_handles_empty_and_disliked_only() {
        let mut rng = StdRng::seed_from_u64(1);
        assert!(pick(&[], NOW, None, &mut rng).is_none());
        let disliked = row("d", 400, Rating::Disliked, None);
        assert!(pick(&[&disliked], NOW, None, &mut rng).is_none());
    }

    #[test]
    fn pick_walks_the_cumulative_weights() {
        let a = row("a", 400, Rating::Unrated, None);
        let skip = row("skip", 400, Rating::Disliked, None);
        let b = row("b", 400, Rating::Liked, None);
        let pool = [&a, &skip, &b];
        assert_eq!(pick(&pool, NOW, None, &mut ConstRng(0)).map(|r| r.id.as_str()), Some("a"));
        assert_eq!(pick(&pool, NOW, None, &mut ConstRng(u64::MAX)).map(|r| r.id.as_str()), Some("b"));
    }

    #[test]
    fn pick_weights() {
        // Same age: liked is 3× unrated.
        assert_eq!(pick_weight(&row("l", 0, Rating::Liked, None), NOW, None), 3.0);
        assert_eq!(pick_weight(&row("u", 0, Rating::Unrated, None), NOW, None), 1.0);
        assert_eq!(pick_weight(&row("d", 0, Rating::Disliked, None), NOW, None), 0.0);
        // A year adds one, capped at three years.
        assert!((pick_weight(&row("u", 365, Rating::Unrated, None), NOW, None) - 2.0).abs() < 1e-9);
        assert!((pick_weight(&row("l", 730, Rating::Liked, None), NOW, None) - 9.0).abs() < 1e-9);
        assert_eq!(pick_weight(&row("u", 5000, Rating::Unrated, None), NOW, None), 4.0);
        assert_eq!(pick_weight(&row("future", -10, Rating::Unrated, None), NOW, None), 1.0);
    }

    #[test]
    fn the_active_moods_originals_weigh_three_times_as_much() {
        let rain = row("rain", 0, Rating::Unrated, None);
        let snow = MemoryRow { mood_id: Some("snow".into()), ..row("snow", 0, Rating::Unrated, None) };
        let before_moods = MemoryRow { mood_id: None, ..row("old", 0, Rating::Unrated, None) };
        assert_eq!(pick_weight(&rain, NOW, Some("rain")), 3.0);
        assert_eq!(pick_weight(&snow, NOW, Some("rain")), 1.0, "other moods' still come back");
        assert_eq!(pick_weight(&before_moods, NOW, Some("rain")), 1.0);
        assert_eq!(pick_weight(&rain, NOW, None), 1.0);
        let liked_in_mood = row("l", 365, Rating::Liked, None);
        assert!((pick_weight(&liked_in_mood, NOW, Some("rain")) - 18.0).abs() < 1e-9, "liked × a year × the mood");
        // Shares follow: 3 to 1.
        let pool = [&rain, &snow];
        let mut rng = StdRng::seed_from_u64(11);
        let in_mood = (0..8000).filter(|_| pick(&pool, NOW, Some("rain"), &mut rng).is_some_and(|r| r.id == "rain")).count();
        assert!((in_mood as f64 / 8000.0 - 0.75).abs() < 0.03, "{in_mood}");
    }

    #[test]
    fn pick_frequencies_follow_the_weights() {
        let liked = row("liked", 0, Rating::Liked, None);
        let unrated = row("unrated", 0, Rating::Unrated, None);
        let ancient = row("ancient", 3650, Rating::Unrated, None);
        let pool = [&liked, &unrated, &ancient];
        let mut rng = StdRng::seed_from_u64(7);
        let mut counts = [0_u32; 3];
        for _ in 0..8000 {
            let picked = pick(&pool, NOW, None, &mut rng).map(|r| r.id.as_str());
            match picked {
                Some("liked") => counts[0] += 1,
                Some("unrated") => counts[1] += 1,
                Some("ancient") => counts[2] += 1,
                other => panic!("unexpected pick {other:?}"),
            }
        }
        // Expected shares 3/8, 1/8, 4/8.
        let share = |n: u32| f64::from(n) / 8000.0;
        assert!((share(counts[0]) - 0.375).abs() < 0.03, "{counts:?}");
        assert!((share(counts[1]) - 0.125).abs() < 0.03, "{counts:?}");
        assert!((share(counts[2]) - 0.5).abs() < 0.03, "{counts:?}");
    }

    #[test]
    fn choose_axes_count_follows_surprise_and_axes_are_distinct() {
        let concept = Concept::default();
        let mut rng = StdRng::seed_from_u64(3);
        for (surprise, expected) in [(0.0, 2), (0.49, 2), (0.5, 3), (1.0, 3)] {
            for _ in 0..50 {
                let axes = choose_axes(surprise, &concept, &mut rng);
                assert_eq!(axes.len(), expected, "surprise {surprise}");
                let distinct: HashSet<EchoAxis> = axes.iter().copied().collect();
                assert_eq!(distinct.len(), axes.len(), "{axes:?}");
            }
        }
    }

    #[test]
    fn choose_axes_can_choose_every_axis() {
        let concept = Concept { time_of_day: "night".into(), ..Concept::default() };
        let mut rng = StdRng::seed_from_u64(11);
        let mut seen = HashSet::new();
        for _ in 0..200 {
            seen.extend(choose_axes(0.0, &concept, &mut rng));
        }
        assert_eq!(seen.len(), EchoAxis::ALL.len());
    }

    #[test]
    fn choose_axes_favours_what_the_original_pins_down() {
        let rate = |concept: &Concept, axis: EchoAxis| {
            let mut rng = StdRng::seed_from_u64(5);
            let hits = (0..4000).filter(|_| choose_axes(0.2, concept, &mut rng).contains(&axis)).count();
            hits as f64 / 4000.0
        };
        let bare = Concept::default();
        let at_night = Concept { time_of_day: "night".into(), ..Concept::default() };
        let stormy = Concept { weather: "storm".into(), season: "  ".into(), ..Concept::default() };
        // Two of six equally likely axes: 1/3. Pinned (weight 2 of 7): 11/21 ≈ 0.52.
        assert!((rate(&bare, EchoAxis::TimeOfDay) - 1.0 / 3.0).abs() < 0.04);
        assert!((rate(&at_night, EchoAxis::TimeOfDay) - 11.0 / 21.0).abs() < 0.04);
        assert!((rate(&stormy, EchoAxis::Weather) - 11.0 / 21.0).abs() < 0.04);
        // Whitespace doesn't pin anything.
        assert!((rate(&stormy, EchoAxis::Season) - rate(&stormy, EchoAxis::Viewpoint)).abs() < 0.05);
    }

    #[test]
    fn age_texts() {
        const HOUR: i64 = 3_600;
        let cases = [
            (-5, "less than an hour ago"),
            (0, "less than an hour ago"),
            (HOUR - 1, "less than an hour ago"),
            (HOUR, "an hour ago"),
            (5 * HOUR + 59 * 60, "5 hours ago"),
            (DAY - 1, "23 hours ago"),
            (DAY, "a day ago"),
            (6 * DAY, "6 days ago"),
            (7 * DAY, "a week ago"),
            (10 * DAY, "a week ago"),
            (21 * DAY, "3 weeks ago"),
            (29 * DAY, "4 weeks ago"),
            (30 * DAY, "a month ago"),
            (75 * DAY, "2 months ago"),
            (122 * DAY, "4 months ago"),
            (334 * DAY, "11 months ago"),
            (351 * DAY, "about a year ago"),
            (400 * DAY, "about a year ago"),
            (547 * DAY, "about a year ago"),
            (549 * DAY, "2 years ago"),
            (730 * DAY, "2 years ago"),
            (3653 * DAY, "10 years ago"),
            (i64::MAX, "292271023045 years ago"),
        ];
        for (seconds, expected) in cases {
            assert_eq!(age_text(seconds), expected, "age_text({seconds})");
        }
    }

    #[test]
    fn ratings_that_allow_echoes() {
        assert!(rating_allows(Rating::Liked));
        assert!(rating_allows(Rating::Unrated));
        assert!(!rating_allows(Rating::Disliked));
    }

    #[test]
    fn every_axis_has_an_instruction() {
        for axis in EchoAxis::ALL {
            assert!(axis.instruction().ends_with('.'), "{axis:?}");
        }
    }
}
