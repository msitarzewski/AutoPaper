//! Novelty: how close a candidate concept is to everything the agent has made, weighted by age.
//!
//! `penalty = cosine × w(age)`, where `w = 1` within the quiet period Q and then halves every Q/2.
//! A candidate is too similar when its maximum penalty reaches the threshold. Echoes are checked against
//! everything outside their own lineage (they're *meant* to resemble their original).

use std::collections::{HashMap, HashSet};

use crate::embed::{HashingEmbedder, cosine};
use crate::store::MemoryRow;

/// Default too-similar threshold for bge-small-en-v1.5 cosine (calibrated by the ignored-by-default
/// `novelty_calibration` test against the real model; update both together).
///
/// Measured 2026-10-05 (revision `5c38ec7`, `embed::concept_text` of hand-written concepts), cosine per
/// pair:
///
/// | group | pairs | min | mean | max |
/// |---|---|---|---|---|
/// | near-duplicates: the same scene reworded | 10 | 0.811 | 0.871 | 0.917 |
/// | echoes: same place; weather, time, season, age, viewpoint or medium changed | 12 | 0.770 | 0.854 | 0.901 |
/// | related: shared keywords, a different scene | 14 | 0.619 | 0.696 | 0.846 |
/// | unrelated | 10 | 0.508 | 0.585 | 0.653 |
/// | copies: the original handed back with token edits | 8 | 0.951 | 0.984 | 0.992 |
///
/// 0.79 is below every near-duplicate and above 13 of the 14 related pairs (all ≤ 0.762). The exception,
/// a rainy neon alley vs a rainy neon overpass at night (0.846), shares every keyword, mood and colour:
/// when the Musts describe the whole scene, only setting and subject tell ideas apart, so such pairs
/// read as repeats and the composer's "too close" retry has to move further away. Missing a repeat is
/// worse than one retry, so the threshold sits under the near-duplicates rather than above that pair.
pub const DEFAULT_THRESHOLD: f32 = 0.79;

/// An echo must resemble its original within this cosine band: recognisably related, not the same.
///
/// Calibrated with `DEFAULT_THRESHOLD` (numbers there): 0.72 is below every echo (≥ 0.770) and above every
/// unrelated pair (≤ 0.653); 0.93 is above every echo (≤ 0.901) and below every copy (≥ 0.951). Cosine
/// can't tell an echo from an independent rewording of its original (0.811–0.917, inside the band): both
/// keep the place, which dominates the embedding. So the upper bound catches an original handed back
/// nearly unchanged, not one that was merely rephrased.
pub const ECHO_BAND: (f32, f32) = (0.72, 0.93);

/// Too-similar threshold for `embed::HashingEmbedder` (a bag of word stems, not semantic), measured on the
/// same hand-written pairs by the `novelty_calibration` test's hashing case (it needs no model and always
/// runs):
///
/// | group | pairs | min | mean | max |
/// |---|---|---|---|---|
/// | near-duplicates | 10 | 0.331 | 0.521 | 0.733 |
/// | echoes | 12 | 0.470 | 0.589 | 0.669 |
/// | related | 14 | −0.017 | 0.257 | 0.472 |
/// | unrelated | 10 | −0.061 | 0.158 | 0.351 |
/// | copies | 8 | 0.875 | 0.931 | 0.960 |
///
/// Stem overlap can't recognise a rewording (near-duplicates spread across the related range), so the
/// hashing threshold only catches lexical repeats: 0.70 is above every related and unrelated pair and below
/// every copy. This is why the real model ships with the apps; the fallback keeps memory working, weaker.
pub const HASHING_THRESHOLD: f32 = 0.70;

/// Echo band for `embed::HashingEmbedder`: 0.40 is above every unrelated pair (≤ 0.351) and below every echo
/// (≥ 0.470); 0.85 is above every echo and below every copy (≥ 0.875). Numbers on `HASHING_THRESHOLD`.
pub const HASHING_ECHO_BAND: (f32, f32) = (0.40, 0.85);

// Checked when the crate builds, so a recalibration can't leave the constants incoherent.
const _: () = {
    assert!(0.0 < DEFAULT_THRESHOLD && DEFAULT_THRESHOLD < 1.0, "the threshold is a cosine in (0, 1)");
    assert!(
        0.0 < ECHO_BAND.0 && ECHO_BAND.0 < ECHO_BAND.1 && ECHO_BAND.1 <= 1.0,
        "the echo band is a range of cosines"
    );
    assert!(0.0 < HASHING_THRESHOLD && HASHING_THRESHOLD < 1.0, "the threshold is a cosine in (0, 1)");
    assert!(
        0.0 < HASHING_ECHO_BAND.0 && HASHING_ECHO_BAND.0 < HASHING_ECHO_BAND.1 && HASHING_ECHO_BAND.1 <= 1.0,
        "the echo band is a range of cosines"
    );
};

/// The calibrated too-similar threshold and echo band for one embedding model: cosines from different
/// models live on different scales.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Calibration {
    pub threshold: f32,
    pub echo_band: (f32, f32),
}

impl Calibration {
    /// `HashingEmbedder`'s constants for its model id; the calibrated bge-small-en-v1.5 constants for
    /// everything else (a new model must be calibrated and added here).
    pub fn for_model(model_id: &str) -> Self {
        if model_id == HashingEmbedder::MODEL_ID {
            Self { threshold: HASHING_THRESHOLD, echo_band: HASHING_ECHO_BAND }
        } else {
            Self { threshold: DEFAULT_THRESHOLD, echo_band: ECHO_BAND }
        }
    }

    /// `low ≤ cosine < high`.
    pub fn in_echo_band(&self, cosine_to_original: f32) -> bool {
        (self.echo_band.0..self.echo_band.1).contains(&cosine_to_original)
    }
}

/// How many of the closest remembered generations a report names.
const NEAREST: usize = 5;

const SECONDS_PER_DAY: f64 = 86_400.0;

/// How strict novelty is: the quiet period Q (from Settings) and the too-similar threshold.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NoveltyPolicy {
    pub quiet_days: i64,
    pub threshold: f32,
}

/// How a candidate compares with memory.
#[derive(Debug, Clone, PartialEq)]
pub struct NoveltyReport {
    /// Highest age-weighted similarity to any remembered generation considered (0 when memory is empty).
    pub max_penalty: f32,
    /// Up to 5 closest remembered generations by penalty, highest first: (id, penalty). Ties are broken
    /// by id, so the order is deterministic.
    pub nearest: Vec<(String, f32)>,
}

impl NoveltyReport {
    /// Novel ⇔ `max_penalty < threshold` (too similar ⇔ it reaches the threshold).
    pub fn is_novel(&self, policy: &NoveltyPolicy) -> bool {
        self.max_penalty < policy.threshold
    }

    /// 1 − max_penalty, clamped to 0..=1 (higher is fresher).
    pub fn score(&self) -> f32 {
        (1.0 - self.max_penalty).clamp(0.0, 1.0)
    }
}

/// Age weight: 1 within `quiet_days`, then `0.5^((age − Q) / (Q/2))`. Negative ages count as 0, and a
/// quiet period under 1 day counts as 1 day.
pub fn age_weight(age_days: f64, quiet_days: i64) -> f32 {
    // `max` also maps NaN to 0.
    let age = age_days.max(0.0);
    let quiet = quiet_days.max(1) as f64;
    if age <= quiet {
        return 1.0;
    }
    0.5_f64.powf((age - quiet) / (quiet / 2.0)) as f32
}

/// Assesses `candidate` (unit vector) against `memory`. Rows whose `embedding_model` differs from
/// `model_id`, or whose embedding is empty (or a different length from the candidate's), are skipped.
/// With `exclude_lineage = Some(root_id)`, rows in that lineage (the root itself and anything echoing it,
/// via `echo_of` chains) are skipped; any member's id works, since it is resolved to its root first.
pub fn assess(
    candidate: &[f32],
    model_id: &str,
    memory: &[MemoryRow],
    now: i64,
    policy: &NoveltyPolicy,
    exclude_lineage: Option<&str>,
) -> NoveltyReport {
    let lineage = exclude_lineage.map(|id| {
        let index = LineageIndex::new(memory);
        let root = index.root(id).to_string();
        (index, root)
    });

    let mut scored: Vec<(&str, f32)> = memory
        .iter()
        .filter(|row| {
            row.embedding_model == model_id && !row.embedding.is_empty() && row.embedding.len() == candidate.len()
        })
        .filter(|row| lineage.as_ref().is_none_or(|(index, root)| index.root(&row.id) != root.as_str()))
        .map(|row| {
            let age_days = now.saturating_sub(row.created_at) as f64 / SECONDS_PER_DAY;
            let penalty = cosine(candidate, &row.embedding) * age_weight(age_days, policy.quiet_days);
            (row.id.as_str(), penalty)
        })
        .collect();

    scored.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    NoveltyReport {
        max_penalty: scored.first().map_or(0.0, |(_, penalty)| *penalty),
        nearest: scored.iter().take(NEAREST).map(|(id, penalty)| ((*id).to_string(), *penalty)).collect(),
    }
}

/// The id of the root original of `id`'s lineage (follows `echo_of` until it ends or leaves `memory`):
/// the first id reached that has no `echo_of` or isn't in `memory` (so `id` itself when it isn't
/// remembered, and the shared, forgotten original when echoes outlive it). A cycle — only possible in a
/// corrupt store — resolves to its smallest id, the same answer from every member.
pub fn lineage_root(id: &str, memory: &[MemoryRow]) -> String {
    LineageIndex::new(memory).root(id).to_string()
}

/// Whether an echo candidate's cosine to its original is within `ECHO_BAND` (`low ≤ cosine < high`).
pub fn in_echo_band(cosine_to_original: f32) -> bool {
    (ECHO_BAND.0..ECHO_BAND.1).contains(&cosine_to_original)
}

/// `echo_of` links of all of memory, for resolving many lineage roots in one pass. If an id appears
/// twice, the first row wins.
pub(crate) struct LineageIndex<'a> {
    parent: HashMap<&'a str, Option<&'a str>>,
}

impl<'a> LineageIndex<'a> {
    pub(crate) fn new(memory: &'a [MemoryRow]) -> Self {
        let mut parent = HashMap::with_capacity(memory.len());
        for row in memory {
            parent.entry(row.id.as_str()).or_insert(row.echo_of.as_deref());
        }
        Self { parent }
    }

    /// See [`lineage_root`].
    pub(crate) fn root<'s>(&'s self, id: &'s str) -> &'s str {
        let mut path: Vec<&str> = Vec::new();
        let mut seen: HashSet<&str> = HashSet::new();
        let mut current = id;
        loop {
            if !seen.insert(current) {
                // Back at `current`: the cycle is everything on the path from its first visit.
                let start = path.iter().position(|step| *step == current).unwrap_or(0);
                return path[start..].iter().copied().min().unwrap_or(current);
            }
            path.push(current);
            match self.parent.get(current) {
                Some(Some(next)) if self.parent.contains_key(next) => current = *next,
                // An echo whose original isn't remembered: the lineage is named after that original.
                Some(Some(next)) => return next,
                // Not an echo, or not in memory at all.
                Some(None) | None => return current,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calibration_follows_the_embedding_model() {
        let hashing = Calibration::for_model(HashingEmbedder::MODEL_ID);
        assert_eq!(hashing, Calibration { threshold: HASHING_THRESHOLD, echo_band: HASHING_ECHO_BAND });
        let bge = Calibration::for_model(crate::embed::CandleEmbedder::MODEL_ID);
        assert_eq!(bge, Calibration { threshold: DEFAULT_THRESHOLD, echo_band: ECHO_BAND });
        assert!(bge.in_echo_band(ECHO_BAND.0) && !bge.in_echo_band(ECHO_BAND.1));
        assert_eq!(bge.in_echo_band(0.8), in_echo_band(0.8));
        assert!(hashing.in_echo_band(0.5) && !hashing.in_echo_band(0.9) && !hashing.in_echo_band(0.3));
    }
    use crate::model::Rating;

    const DAY: i64 = 86_400;
    const NOW: i64 = 1_800_000_000;
    const POLICY: NoveltyPolicy = NoveltyPolicy { quiet_days: 182, threshold: 0.82 };

    fn row(id: &str, days_ago: i64, embedding: Vec<f32>, echo_of: Option<&str>) -> MemoryRow {
        MemoryRow {
            id: id.into(),
            created_at: NOW - days_ago * DAY,
            title: format!("title {id}"),
            summary: format!("summary {id}"),
            embedding,
            embedding_model: "test".into(),
            rating: Rating::Unrated,
            echo_of: echo_of.map(Into::into),
            mood_id: None,
        }
    }

    /// A unit vector at `cos` to [1, 0].
    fn at(cos: f32) -> Vec<f32> {
        vec![cos, (1.0 - cos * cos).max(0.0).sqrt()]
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn age_weight_is_flat_then_halves_every_half_quiet_period() {
        assert_eq!(age_weight(0.0, 182), 1.0);
        assert_eq!(age_weight(182.0, 182), 1.0);
        assert!(close(age_weight(182.0 + 91.0, 182), 0.5));
        assert!(close(age_weight(182.0 + 182.0, 182), 0.25));
        assert!(close(age_weight(30.0 + 45.0, 30), 0.125));
        assert!(age_weight(182.5, 182) < 1.0);
    }

    #[test]
    fn age_weight_edge_cases() {
        assert_eq!(age_weight(-10.0, 182), 1.0);
        assert_eq!(age_weight(f64::NAN, 182), 1.0);
        // Q < 1 day counts as 1 day: flat for a day, then halving every half day.
        assert_eq!(age_weight(1.0, 0), 1.0);
        assert!(close(age_weight(1.5, 0), 0.5));
        assert!(close(age_weight(1.5, -5), 0.5));
        assert_eq!(age_weight(1e9, 30), 0.0);
    }

    #[test]
    fn report_novelty_and_score() {
        let report = |max_penalty| NoveltyReport { max_penalty, nearest: vec![] };
        assert!(report(0.81).is_novel(&POLICY));
        assert!(!report(0.82).is_novel(&POLICY));
        assert!(!report(0.9).is_novel(&POLICY));
        assert!(close(report(0.3).score(), 0.7));
        assert_eq!(report(-0.2).score(), 1.0);
        assert_eq!(report(1.0).score(), 0.0);
    }

    #[test]
    fn empty_memory_is_perfectly_novel() {
        let report = assess(&at(1.0), "test", &[], NOW, &POLICY, None);
        assert_eq!(report, NoveltyReport { max_penalty: 0.0, nearest: vec![] });
        assert!(report.is_novel(&POLICY));
        assert_eq!(report.score(), 1.0);
    }

    #[test]
    fn penalty_is_cosine_times_age_weight() {
        let memory = vec![
            row("recent", 10, at(0.9), None),
            // 0.95 cosine, but one half-quiet-period past Q → 0.475.
            row("old", 182 + 91, at(0.95), None),
        ];
        let report = assess(&at(1.0), "test", &memory, NOW, &POLICY, None);
        assert!(close(report.max_penalty, 0.9));
        assert_eq!(report.nearest[0].0, "recent");
        assert_eq!(report.nearest[1].0, "old");
        assert!(close(report.nearest[1].1, 0.475));
        assert!(!report.is_novel(&POLICY));
    }

    #[test]
    fn an_old_near_duplicate_fades_back_into_novelty() {
        let memory = vec![row("dup", 182 + 182, at(0.99), None)];
        let report = assess(&at(1.0), "test", &memory, NOW, &POLICY, None);
        assert!(close(report.max_penalty, 0.99 * 0.25));
        assert!(report.is_novel(&POLICY));
    }

    #[test]
    fn nearest_keeps_top_five_in_order_with_id_tiebreak() {
        let cosines = [("g", 0.1), ("b", 0.5), ("a", 0.5), ("c", 0.7), ("d", 0.2), ("e", 0.3), ("f", 0.4)];
        let memory: Vec<MemoryRow> = cosines
            .into_iter()
            .map(|(id, cos)| row(id, 1, at(cos), None))
            .collect();
        let report = assess(&at(1.0), "test", &memory, NOW, &POLICY, None);
        let ids: Vec<&str> = report.nearest.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, ["c", "a", "b", "f", "e"]);
        assert!(close(report.max_penalty, 0.7));
    }

    #[test]
    fn skips_other_models_empty_and_mismatched_embeddings() {
        let mut other_model = row("other", 1, at(1.0), None);
        other_model.embedding_model = "hashing-v1".into();
        let memory = vec![
            other_model,
            row("empty", 1, vec![], None),
            row("short", 1, vec![1.0], None),
            row("kept", 1, at(0.4), None),
        ];
        let report = assess(&at(1.0), "test", &memory, NOW, &POLICY, None);
        assert_eq!(report.nearest.len(), 1);
        assert_eq!(report.nearest[0].0, "kept");
    }

    #[test]
    fn negative_similarity_is_reported_honestly() {
        let memory = vec![row("opposite", 1, vec![-1.0, 0.0], None)];
        let report = assess(&[1.0, 0.0], "test", &memory, NOW, &POLICY, None);
        assert!(close(report.max_penalty, -1.0));
        assert_eq!(report.score(), 1.0);
    }

    #[test]
    fn future_rows_count_as_brand_new() {
        let memory = vec![row("skewed", -3, at(0.9), None)];
        let report = assess(&at(1.0), "test", &memory, NOW, &POLICY, None);
        assert!(close(report.max_penalty, 0.9));
    }

    fn lineage_memory() -> Vec<MemoryRow> {
        vec![
            row("root", 400, at(0.99), None),
            row("echo1", 200, at(0.98), Some("root")),
            row("echo2", 10, at(0.97), Some("echo1")),
            row("orphan", 50, at(0.6), Some("forgotten")),
            row("orphan2", 40, at(0.5), Some("forgotten")),
            row("other", 5, at(0.7), None),
        ]
    }

    #[test]
    fn lineage_root_follows_echo_chains() {
        let memory = lineage_memory();
        assert_eq!(lineage_root("root", &memory), "root");
        assert_eq!(lineage_root("echo1", &memory), "root");
        assert_eq!(lineage_root("echo2", &memory), "root");
        assert_eq!(lineage_root("other", &memory), "other");
        assert_eq!(lineage_root("unknown", &memory), "unknown");
        // Echoes of a forgotten original share that original as their root.
        assert_eq!(lineage_root("orphan", &memory), "forgotten");
        assert_eq!(lineage_root("orphan2", &memory), "forgotten");
    }

    #[test]
    fn lineage_root_survives_cycles() {
        let memory = vec![
            row("b", 1, at(1.0), Some("c")),
            row("c", 1, at(1.0), Some("a")),
            row("a", 1, at(1.0), Some("b")),
            row("tail", 1, at(1.0), Some("b")),
            row("self", 1, at(1.0), Some("self")),
        ];
        for id in ["a", "b", "c", "tail"] {
            assert_eq!(lineage_root(id, &memory), "a", "from {id}");
        }
        assert_eq!(lineage_root("self", &memory), "self");
    }

    #[test]
    fn excluding_a_lineage_skips_root_and_all_echoes() {
        let memory = lineage_memory();
        let report = assess(&at(1.0), "test", &memory, NOW, &POLICY, Some("root"));
        let ids: Vec<&str> = report.nearest.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, ["other", "orphan", "orphan2"]);
        assert!(close(report.max_penalty, 0.7));

        // Any member's id names the same lineage.
        assert_eq!(assess(&at(1.0), "test", &memory, NOW, &POLICY, Some("echo2")), report);

        let report = assess(&at(1.0), "test", &memory, NOW, &POLICY, Some("forgotten"));
        let ids: Vec<&str> = report.nearest.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, ["echo2", "echo1", "other", "root"]);
    }

    #[test]
    fn echo_band_is_half_open() {
        assert!(in_echo_band(ECHO_BAND.0));
        assert!(in_echo_band((ECHO_BAND.0 + ECHO_BAND.1) / 2.0));
        assert!(!in_echo_band(ECHO_BAND.1));
        assert!(!in_echo_band(ECHO_BAND.0 - 0.01));
        assert!(!in_echo_band(f32::NAN));
    }

}
