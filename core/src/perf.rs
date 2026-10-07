//! Performance history: how long provider calls take on this computer, and what that says about the next one.
//!
//! Every finished provider call is recorded (`Timing`: job, provider, server origin, model, size, steps,
//! seconds, when) — kept locally, never sent. `estimate` turns the recorded calls of one kind (same job, provider,
//! origin and model) into a number: a recency-weighted median (each call weighs half as much per `HALF_LIFE_DAYS`
//! of age) of their seconds, each first scaled to the size asked about (by pixels: diffusion time grows with the
//! picture's area) and, when both are known, to its sampler steps. Nothing recorded → no estimate, never a
//! made-up number.
//!
//! `PaintTracker` turns a painting under way into a fraction and seconds left (`ProgressDetail`): from the
//! painter's own step progress when it reports any (ComfyUI), at the pace measured in this run; otherwise from the
//! learned estimate and the time so far; otherwise nothing (hosts show an indeterminate indicator). For a painter
//! that reports steps, time alone fills at most half the bar before the first step: when that step comes later
//! than the estimate said (a cold model load after warm runs), the bar waits at half instead of near the top, and
//! the steps fill the rest at their own pace — the fraction never goes back.

use std::time::Duration;

use tokio::time::Instant;

use crate::model::{ProviderJob, ProviderKind};

/// How fast old timings fade from an estimate: a call this many days old counts half.
pub const HALF_LIFE_DAYS: f64 = 30.0;
/// The fraction never passes this before the picture has arrived (decoding and saving still to come).
pub const MAX_FRACTION_WHILE_PAINTING: f32 = 0.99;
/// Time alone (no step progress) never shows more than this: the estimate may be short.
const MAX_FRACTION_BY_TIME: f32 = 0.95;
/// Before the first step of a painter that reports steps, time alone shows no more than this: the steps own the
/// rest of the bar, so a first step that comes late doesn't leave them only the last few percent to fill.
const MAX_FRACTION_BEFORE_STEPS: f32 = 0.5;
const SECONDS_PER_DAY: f64 = 86_400.0;

/// One finished provider call.
#[derive(Debug, Clone, PartialEq)]
pub struct Timing {
    pub job: ProviderJob,
    pub provider: ProviderKind,
    /// The server's origin for local and OpenAI-compatible providers (`http://127.0.0.1:8188`: local servers can
    /// be other machines); empty for hosted services and Demo.
    pub origin: String,
    /// The model asked for (the provider's default resolved), as the next call would ask.
    pub model: String,
    /// The size asked for; 0 × 0 for text.
    pub width: u32,
    pub height: u32,
    /// Sampler steps of the graph that ran, when the provider knows them (ComfyUI).
    pub steps: Option<u32>,
    pub seconds: f64,
    /// Unix seconds.
    pub finished_at: i64,
}

/// Seconds a call like the recorded ones would take now for `width` × `height` (0 × 0: the sizes don't matter,
/// as for text) and `steps` (when known on both sides), or `None` when nothing is recorded.
pub fn estimate(timings: &[Timing], now: i64, width: u32, height: u32, steps: Option<u32>) -> Option<f64> {
    let pixels = f64::from(width) * f64::from(height);
    let mut samples: Vec<(f64, f64)> = timings
        .iter()
        .filter(|timing| timing.seconds.is_finite() && timing.seconds >= 0.0)
        .map(|timing| {
            let recorded = f64::from(timing.width) * f64::from(timing.height);
            let by_size = if pixels > 0.0 && recorded > 0.0 { pixels / recorded } else { 1.0 };
            let by_steps = match (steps, timing.steps) {
                (Some(wanted), Some(ran)) if wanted > 0 && ran > 0 => f64::from(wanted) / f64::from(ran),
                _ => 1.0,
            };
            let age_days = (now.saturating_sub(timing.finished_at)).max(0) as f64 / SECONDS_PER_DAY;
            (timing.seconds * by_size * by_steps, 0.5f64.powf(age_days / HALF_LIFE_DAYS))
        })
        .filter(|(_, weight)| *weight > 0.0)
        .collect();
    weighted_median(&mut samples)
}

/// The value where the cumulative weight (in value order) reaches half the total; between two values when it
/// lands exactly on a boundary.
fn weighted_median(samples: &mut [(f64, f64)]) -> Option<f64> {
    let total: f64 = samples.iter().map(|(_, weight)| weight).sum();
    if samples.is_empty() || total.is_nan() || total <= 0.0 {
        return None;
    }
    samples.sort_by(|a, b| a.0.total_cmp(&b.0));
    let half = total / 2.0;
    let mut cumulative = 0.0;
    for (index, (value, weight)) in samples.iter().enumerate() {
        cumulative += weight;
        if (cumulative - half).abs() <= total * 1e-12 {
            return Some(samples.get(index + 1).map_or(*value, |(next, _)| (value + next) / 2.0));
        }
        if cumulative > half {
            return Some(*value);
        }
    }
    samples.last().map(|(value, _)| *value)
}

/// Whole seconds for hosts: rounded, at least 1.
pub fn whole_seconds(seconds: f64) -> u32 {
    if seconds.is_finite() && seconds > 0.0 { seconds.round().clamp(1.0, f64::from(u32::MAX)) as u32 } else { 1 }
}

/// Where a painting is, as a fraction and seconds left (see the module docs).
#[derive(Debug, Clone)]
pub struct PaintTracker {
    started: Instant,
    /// The learned estimate for the whole call, in seconds.
    estimate: Option<f64>,
    /// The most time alone may show before the first step (`MAX_FRACTION_BEFORE_STEPS` for painters that report
    /// steps, else `MAX_FRACTION_BY_TIME`).
    time_cap: f32,
    /// The fraction shown when the first step came: steps then fill the rest, so the ring never jumps back.
    base: f32,
    first_step: Option<(Instant, u32)>,
    last_step: Option<(Instant, u32, u32)>,
    /// The highest fraction reported so far.
    shown: Option<f32>,
}

impl PaintTracker {
    /// `reports_steps`: the painter tells its sampler steps as they happen (ComfyUI, Demo's slow mode).
    pub fn new(started: Instant, estimate: Option<f64>, reports_steps: bool) -> Self {
        let estimate = estimate.filter(|seconds| seconds.is_finite() && *seconds > 0.0);
        let time_cap = if reports_steps { MAX_FRACTION_BEFORE_STEPS } else { MAX_FRACTION_BY_TIME };
        Self { started, estimate, time_cap, base: 0.0, first_step: None, last_step: None, shown: None }
    }

    /// The painter finished `done` of `total` sampler steps at `at`.
    pub fn step(&mut self, at: Instant, done: u32, total: u32) {
        if total == 0 {
            return;
        }
        let done = done.min(total);
        if self.first_step.is_none() {
            self.base = self.shown.unwrap_or(0.0);
            self.first_step = Some((at, done));
        }
        self.last_step = Some((at, done, total));
    }

    /// (fraction, seconds left) at `now`.
    pub fn at(&mut self, now: Instant) -> (Option<f32>, Option<u32>) {
        let elapsed = now.saturating_duration_since(self.started).as_secs_f64();
        let by_estimate = self.estimate.map(|estimate| estimate - elapsed);
        let (fraction, seconds_left) = match self.last_step {
            Some((at, done, total)) => {
                let fraction = self.base + (1.0 - self.base) * (done as f32 / total as f32);
                let paced = self.first_step.and_then(|(first_at, first_done)| {
                    (done > first_done).then(|| {
                        let per_step = at.saturating_duration_since(first_at).as_secs_f64() / f64::from(done - first_done);
                        let since = now.saturating_duration_since(at).as_secs_f64();
                        (per_step * f64::from(total - done) - since).max(0.0)
                    })
                });
                (Some(fraction), paced.or(by_estimate.filter(|left| *left > 0.0)))
            }
            None => {
                let fraction = self.estimate.map(|estimate| ((elapsed / estimate) as f32).min(self.time_cap));
                (fraction, by_estimate.filter(|left| *left > 0.0))
            }
        };
        let fraction = fraction.map(|fraction| {
            let fraction = fraction.max(self.shown.unwrap_or(0.0)).clamp(0.0, MAX_FRACTION_WHILE_PAINTING);
            self.shown = Some(fraction);
            fraction
        });
        (fraction, seconds_left.map(|left| left.ceil().min(f64::from(u32::MAX)) as u32))
    }
}

/// `Duration` from seconds, for timeouts derived from estimates (negative or not finite → zero).
pub fn duration_secs(seconds: f64) -> Duration {
    if seconds.is_finite() && seconds > 0.0 { Duration::from_secs_f64(seconds.min(1e9)) } else { Duration::ZERO }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_791_158_400;
    const DAY: i64 = 86_400;

    fn timing(seconds: f64, days_ago: i64, width: u32, height: u32, steps: Option<u32>) -> Timing {
        Timing {
            job: ProviderJob::Images,
            provider: ProviderKind::ComfyUi,
            origin: "http://127.0.0.1:8188".into(),
            model: "z_image_turbo_bf16.safetensors".into(),
            width,
            height,
            steps,
            seconds,
            finished_at: NOW - days_ago * DAY,
        }
    }

    #[test]
    fn no_timings_no_estimate() {
        assert_eq!(estimate(&[], NOW, 2048, 1152, Some(8)), None);
    }

    #[test]
    fn one_timing_is_the_estimate_at_its_own_size() {
        let recorded = [timing(62.0, 0, 2048, 1152, Some(8))];
        assert_eq!(estimate(&recorded, NOW, 2048, 1152, Some(8)), Some(62.0));
        // Twice the pixels → twice the time; a quarter → a quarter.
        assert_eq!(estimate(&recorded, NOW, 4096, 1152, Some(8)), Some(124.0));
        assert_eq!(estimate(&recorded, NOW, 1024, 576, Some(8)), Some(15.5));
        // Steps scale when both sides know them.
        assert_eq!(estimate(&recorded, NOW, 2048, 1152, Some(16)), Some(124.0));
        assert_eq!(estimate(&recorded, NOW, 2048, 1152, None), Some(62.0));
        // Text: no size.
        let text = Timing { job: ProviderJob::Concepts, width: 0, height: 0, steps: None, ..timing(3.5, 0, 0, 0, None) };
        assert_eq!(estimate(&[text], NOW, 0, 0, None), Some(3.5));
    }

    #[test]
    fn the_median_ignores_one_slow_outlier_and_recent_calls_weigh_more() {
        // A first run that loaded the model (140 s) among warm runs doesn't move it.
        let warm = [
            timing(140.0, 3, 2048, 1152, Some(8)),
            timing(61.0, 2, 2048, 1152, Some(8)),
            timing(63.0, 1, 2048, 1152, Some(8)),
            timing(62.0, 0, 2048, 1152, Some(8)),
        ];
        assert_eq!(estimate(&warm, NOW, 2048, 1152, Some(8)), Some(62.0));
        // Equal weights split exactly in half: between the middle two.
        let same_day: Vec<Timing> = warm.iter().map(|t| Timing { finished_at: NOW, ..t.clone() }).collect();
        assert_eq!(estimate(&same_day, NOW, 2048, 1152, Some(8)), Some(62.5));
        // Old slow calls (another machine's ComfyUI before an upgrade) fade: three at 120 s two to three months
        // ago weigh less than two at 60 s this week.
        let upgraded = [
            timing(120.0, 90, 2048, 1152, None),
            timing(121.0, 80, 2048, 1152, None),
            timing(119.0, 70, 2048, 1152, None),
            timing(60.0, 2, 2048, 1152, None),
            timing(61.0, 1, 2048, 1152, None),
        ];
        assert_eq!(estimate(&upgraded, NOW, 2048, 1152, None), Some(61.0));
    }

    #[test]
    fn whole_seconds_round_to_at_least_one() {
        assert_eq!(whole_seconds(61.2), 61);
        assert_eq!(whole_seconds(61.5), 62);
        assert_eq!(whole_seconds(0.01), 1);
        assert_eq!(whole_seconds(f64::NAN), 1);
    }

    #[test]
    fn without_steps_or_an_estimate_progress_is_unknown() {
        let start = Instant::now();
        let mut tracker = PaintTracker::new(start, None, true);
        assert_eq!(tracker.at(start + Duration::from_secs(30)), (None, None));
    }

    #[test]
    fn an_estimate_alone_gives_time_based_progress_that_never_claims_done() {
        let start = Instant::now();
        // A painter without steps (a hosted service): time is all there is.
        let mut tracker = PaintTracker::new(start, Some(100.0), false);
        assert_eq!(tracker.at(start), (Some(0.0), Some(100)));
        assert_eq!(tracker.at(start + Duration::from_secs(25)), (Some(0.25), Some(75)));
        // Past the estimate: the ring waits short of full, and there's nothing honest to say about time.
        assert_eq!(tracker.at(start + Duration::from_secs(150)), (Some(MAX_FRACTION_BY_TIME), None));
    }

    #[test]
    fn steps_take_over_from_time_without_going_back_and_pace_the_rest() {
        let start = Instant::now();
        let mut tracker = PaintTracker::new(start, Some(100.0), true);
        // 40 s of loading at the estimate's pace: 40%.
        assert_eq!(tracker.at(start + Duration::from_secs(40)).0, Some(0.4));
        // Steps then fill the remaining 60%: step 1 of 8 is 0.4 + 0.6/8.
        tracker.step(start + Duration::from_secs(45), 1, 8);
        let (fraction, left) = tracker.at(start + Duration::from_secs(45));
        assert!((fraction.unwrap() - 0.475).abs() < 1e-6, "{fraction:?}");
        assert_eq!(left, Some(55), "one step says nothing about pace yet: the estimate speaks");
        // Steps 1→5 took 20 s: 5 s a step, 3 steps left = 15 s.
        tracker.step(start + Duration::from_secs(65), 5, 8);
        let (fraction, left) = tracker.at(start + Duration::from_secs(65));
        assert!((fraction.unwrap() - 0.775).abs() < 1e-6, "{fraction:?}");
        assert_eq!(left, Some(15));
        assert_eq!(tracker.at(start + Duration::from_secs(70)).1, Some(10), "counting down between steps");
        tracker.step(start + Duration::from_secs(80), 8, 8);
        assert_eq!(tracker.at(start + Duration::from_secs(80)), (Some(MAX_FRACTION_WHILE_PAINTING), Some(0)));
    }

    #[test]
    fn steps_without_an_estimate_still_give_a_fraction() {
        let start = Instant::now();
        let mut tracker = PaintTracker::new(start, None, true);
        tracker.step(start + Duration::from_secs(10), 2, 8);
        assert_eq!(tracker.at(start + Duration::from_secs(10)), (Some(0.25), None));
        tracker.step(start + Duration::from_secs(20), 4, 8);
        assert_eq!(tracker.at(start + Duration::from_secs(20)), (Some(0.5), Some(20)));
        // A second sampler (a refiner) reporting its own count can't move the ring back.
        tracker.step(start + Duration::from_secs(25), 1, 8);
        assert_eq!(tracker.at(start + Duration::from_secs(25)).0, Some(0.5));
    }

    #[test]
    fn a_first_step_later_than_the_estimate_leaves_the_steps_half_the_bar() {
        // Learned from warm runs: 2 s. This run loads the model first (10 s), then paints 8 steps 3 s apart.
        let start = Instant::now();
        let mut tracker = PaintTracker::new(start, Some(2.0), true);
        assert_eq!(tracker.at(start + Duration::from_millis(500)), (Some(0.25), Some(2)));
        // Past the estimate with no step yet: the bar waits at half, with nothing honest to say about time.
        assert_eq!(tracker.at(start + Duration::from_secs(2)), (Some(MAX_FRACTION_BEFORE_STEPS), None));
        assert_eq!(tracker.at(start + Duration::from_secs(9)), (Some(MAX_FRACTION_BEFORE_STEPS), None));
        let mut last = MAX_FRACTION_BEFORE_STEPS;
        for done in 1..=8u32 {
            let at = start + Duration::from_secs(10 + 3 * u64::from(done - 1));
            tracker.step(at, done, 8);
            let (fraction, left) = tracker.at(at);
            let fraction = fraction.unwrap();
            // Each step moves the bar by the same share of the half the steps own (the last one stops short of
            // full until the picture arrives), and once there's a pace the time left follows it.
            let expected = (0.5 + 0.5 * done as f32 / 8.0).min(MAX_FRACTION_WHILE_PAINTING);
            assert!((fraction - expected).abs() < 1e-6, "step {done}: {fraction}");
            assert!(fraction >= last, "step {done}: never backwards");
            last = fraction;
            if done >= 2 {
                assert_eq!(left, Some(3 * (8 - done)), "step {done}");
            }
            // Never near the top while tens of seconds are left.
            if left.is_some_and(|left| left >= 10) {
                assert!(fraction < 0.9, "step {done}: {fraction} with {left:?} s left");
            }
        }
    }
}
