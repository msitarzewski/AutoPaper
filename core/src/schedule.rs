//! When the next wallpaper is due, budget checks and failure backoff. Pure functions; the engine stores
//! the inputs (when the schedule last restarted, backoff, spend).

use crate::model::{Cadence, Settings};

const SECONDS_PER_DAY: i64 = 86_400;

/// First retry after a transient failure; each further failure doubles it, up to `MAX_BACKOFF_SECS`.
const FIRST_BACKOFF_SECS: i64 = 10 * 60;
const MAX_BACKOFF_SECS: i64 = 60 * 60;

/// None when paused or `Cadence::Manual`. Otherwise `last_slot + interval` (or `now` if nothing has been
/// made yet), but never before `backoff_until`. `last_slot` is when the schedule last restarted: the newest
/// new wallpaper of any trigger, or a slot skipped over budget. A `last_slot` later than `now` (the clock was
/// set back) counts as `now`; the engine stores that `now` as the last slot, so the schedule waits at most one
/// interval instead of stalling until the clock catches up.
pub fn next_due(settings: &Settings, last_slot: Option<i64>, backoff_until: Option<i64>, now: i64) -> Option<i64> {
    next_due_after(settings, last_slot, 0, backoff_until, now)
}

/// `next_due` for a last slot filled by a scheduled wallpaper that started `lead` seconds before it was due (it
/// started early by its estimated duration): the next is one interval after the time it was due, so early starts
/// don't drift the schedule earlier. `last_slot` itself is when it started (never in the future, so a clock set
/// back is still told by `last_slot > now`).
pub fn next_due_after(
    settings: &Settings,
    last_slot: Option<i64>,
    lead: i64,
    backoff_until: Option<i64>,
    now: i64,
) -> Option<i64> {
    if settings.paused {
        return None;
    }
    let interval = settings.cadence.interval_secs()?;
    let due = match last_slot {
        Some(last) => last.min(now).saturating_add(lead.clamp(0, interval)).saturating_add(interval),
        None => now,
    };
    Some(backoff_until.map_or(due, |until| due.max(until)))
}

pub fn is_due(next: Option<i64>, now: i64) -> bool {
    next.is_some_and(|due| due <= now)
}

/// Seconds to wait after `consecutive_failures` transient failures: 0 for 0, then 10 min, 20 min,
/// 40 min, capped at 1 hour.
pub fn backoff_secs(consecutive_failures: u32) -> i64 {
    match consecutive_failures {
        0 => 0,
        n => (FIRST_BACKOFF_SECS << (n - 1).min(3)).min(MAX_BACKOFF_SECS),
    }
}

/// "YYYY-MM" of a Unix time, in UTC (budget months are UTC months, documented in the UI as "this month").
pub fn month_key(unix: i64) -> String {
    let (year, month) = civil_year_month(unix.div_euclid(SECONDS_PER_DAY));
    format!("{year:04}-{month:02}")
}

/// True when there's no cap, or spent + next stays within the cap (cents → micro-USD × 10,000).
pub fn budget_allows(budget_cents: Option<u32>, spent_microusd: u64, next_cost_microusd: u64) -> bool {
    budget_cents.is_none_or(|cents| spent_microusd.saturating_add(next_cost_microusd) <= u64::from(cents) * 10_000)
}

/// Scheduled wallpapers in a 30-day month at this cadence (0 for Manual).
pub fn runs_per_month(cadence: Cadence) -> f64 {
    cadence.interval_secs().map_or(0.0, |interval| (30 * SECONDS_PER_DAY) as f64 / interval as f64)
}

/// Proleptic Gregorian (year, month) of a day count since 1970-01-01 (Howard Hinnant's
/// `civil_from_days`). Total over every `i64` day count a Unix time in seconds can produce.
fn civil_year_month(days: i64) -> (i64, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097); // 0..=146_096
    let year_of_era = (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365; // 0..=399
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100); // 0..=365
    let shifted_month = (5 * day_of_year + 2) / 153; // 0..=11, March first
    let month = if shifted_month < 10 { shifted_month + 3 } else { shifted_month - 9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    // `month` is 1..=12 by construction.
    (year, month as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: i64 = 3600;
    const DAY: i64 = 24 * HOUR;
    const NOW: i64 = 1_790_000_000;

    fn settings(cadence: Cadence) -> Settings {
        Settings { cadence, ..Settings::default() }
    }

    #[test]
    fn paused_and_manual_are_never_due() {
        let paused = Settings { paused: true, ..Settings::default() };
        assert_eq!(next_due(&paused, Some(NOW - 10 * DAY), None, NOW), None);
        assert_eq!(next_due(&settings(Cadence::Manual), Some(NOW - 10 * DAY), None, NOW), None);
        assert_eq!(next_due(&settings(Cadence::Manual), None, Some(NOW + HOUR), NOW), None);
        assert!(!is_due(None, NOW));
    }

    #[test]
    fn a_slot_filled_early_counts_from_when_it_was_due() {
        let started = NOW - 3 * HOUR;
        let daily = settings(Cadence::Daily);
        assert_eq!(next_due_after(&daily, Some(started), 300, None, NOW), Some(started + 300 + DAY));
        assert_eq!(next_due_after(&daily, Some(started), 0, None, NOW), next_due(&daily, Some(started), None, NOW));
        // Never more than an interval of lead (a corrupt value), never negative.
        assert_eq!(next_due_after(&settings(Cadence::Hourly), Some(started), 10 * DAY, None, NOW), Some(started + 2 * HOUR));
        assert_eq!(next_due_after(&daily, Some(started), -50, None, NOW), Some(started + DAY));
    }

    #[test]
    fn first_run_is_due_now() {
        let next = next_due(&settings(Cadence::Daily), None, None, NOW);
        assert_eq!(next, Some(NOW));
        assert!(is_due(next, NOW));
    }

    #[test]
    fn first_run_waits_for_backoff() {
        assert_eq!(next_due(&settings(Cadence::Daily), None, Some(NOW + 600), NOW), Some(NOW + 600));
        // A backoff already in the past doesn't delay it.
        assert_eq!(next_due(&settings(Cadence::Daily), None, Some(NOW - 600), NOW), Some(NOW));
    }

    #[test]
    fn due_one_interval_after_the_last_scheduled_run() {
        let last = NOW - 5 * HOUR;
        assert_eq!(next_due(&settings(Cadence::Hourly), Some(last), None, NOW), Some(last + HOUR));
        assert_eq!(next_due(&settings(Cadence::Every3Hours), Some(last), None, NOW), Some(last + 3 * HOUR));
        assert_eq!(next_due(&settings(Cadence::Every6Hours), Some(last), None, NOW), Some(last + 6 * HOUR));
        assert_eq!(next_due(&settings(Cadence::Every12Hours), Some(last), None, NOW), Some(last + 12 * HOUR));
        assert_eq!(next_due(&settings(Cadence::Daily), Some(last), None, NOW), Some(last + DAY));
        assert_eq!(next_due(&settings(Cadence::Weekly), Some(last), None, NOW), Some(last + 7 * DAY));
    }

    #[test]
    fn overdue_stays_in_the_past_so_it_runs_now() {
        let next = next_due(&settings(Cadence::Daily), Some(NOW - 3 * DAY), None, NOW);
        assert_eq!(next, Some(NOW - 2 * DAY));
        assert!(is_due(next, NOW));
    }

    #[test]
    fn backoff_later_than_due_wins() {
        let last = NOW - DAY;
        let next = next_due(&settings(Cadence::Daily), Some(last), Some(NOW + 1200), NOW);
        assert_eq!(next, Some(NOW + 1200));
        assert!(!is_due(next, NOW));
        assert!(is_due(next, NOW + 1200));
    }

    #[test]
    fn backoff_earlier_than_due_is_ignored() {
        let last = NOW - HOUR;
        assert_eq!(next_due(&settings(Cadence::Daily), Some(last), Some(NOW + 600), NOW), Some(last + DAY));
    }

    #[test]
    fn last_run_in_the_future_counts_as_now() {
        assert_eq!(next_due(&settings(Cadence::Daily), Some(NOW + 30 * DAY), None, NOW), Some(NOW + DAY));
    }

    #[test]
    fn extreme_times_saturate() {
        assert_eq!(next_due(&settings(Cadence::Weekly), Some(i64::MAX), None, i64::MAX), Some(i64::MAX));
    }

    #[test]
    fn is_due_at_the_exact_second() {
        assert!(is_due(Some(NOW), NOW));
        assert!(is_due(Some(NOW - 1), NOW));
        assert!(!is_due(Some(NOW + 1), NOW));
    }

    #[test]
    fn backoff_doubles_to_an_hour() {
        assert_eq!(backoff_secs(0), 0);
        assert_eq!(backoff_secs(1), 10 * 60);
        assert_eq!(backoff_secs(2), 20 * 60);
        assert_eq!(backoff_secs(3), 40 * 60);
        assert_eq!(backoff_secs(4), 60 * 60);
        assert_eq!(backoff_secs(5), 60 * 60);
        assert_eq!(backoff_secs(u32::MAX), 60 * 60);
    }

    #[test]
    fn month_keys_are_utc_months() {
        assert_eq!(month_key(0), "1970-01");
        assert_eq!(month_key(-1), "1969-12");
        assert_eq!(month_key(951_782_399), "2000-02"); // 2000-02-28T23:59:59Z
        assert_eq!(month_key(951_868_799), "2000-02"); // 2000-02-29T23:59:59Z (leap)
        assert_eq!(month_key(951_868_800), "2000-03");
        assert_eq!(month_key(1_709_251_199), "2024-02"); // 2024-02-29T23:59:59Z
        assert_eq!(month_key(1_709_251_200), "2024-03");
        assert_eq!(month_key(1_798_761_599), "2026-12"); // 2026-12-31T23:59:59Z
        assert_eq!(month_key(1_798_761_600), "2027-01");
    }

    #[test]
    fn month_keys_agree_with_chrono() {
        // Deterministic sweep: every 9 days 7 hours 13 minutes from 1900 to 2200.
        let mut t = -2_208_988_800; // 1900-01-01
        while t < 7_258_118_400 {
            let expected = chrono::DateTime::from_timestamp(t, 0).map(|d| d.format("%Y-%m").to_string());
            assert_eq!(Some(month_key(t)), expected, "t = {t}");
            t += 9 * DAY + 7 * HOUR + 13 * 60;
        }
    }

    #[test]
    fn month_key_never_panics_at_the_extremes() {
        assert!(!month_key(i64::MAX).is_empty());
        assert!(!month_key(i64::MIN).is_empty());
    }

    #[test]
    fn budget_boundaries() {
        assert!(budget_allows(None, u64::MAX, u64::MAX));
        // $5.00 = 500 cents = 5,000,000 µ$.
        assert!(budget_allows(Some(500), 4_900_000, 100_000));
        assert!(!budget_allows(Some(500), 4_900_001, 100_000));
        assert!(budget_allows(Some(500), 0, 0));
        assert!(!budget_allows(Some(0), 0, 1));
        assert!(budget_allows(Some(0), 0, 0));
        assert!(!budget_allows(Some(u32::MAX), u64::MAX, 1));
    }

    #[test]
    fn runs_per_month_by_cadence() {
        assert_eq!(runs_per_month(Cadence::Hourly), 720.0);
        assert_eq!(runs_per_month(Cadence::Every3Hours), 240.0);
        assert_eq!(runs_per_month(Cadence::Every6Hours), 120.0);
        assert_eq!(runs_per_month(Cadence::Every12Hours), 60.0);
        assert_eq!(runs_per_month(Cadence::Daily), 30.0);
        assert!((runs_per_month(Cadence::Weekly) - 30.0 / 7.0).abs() < 1e-12);
        assert_eq!(runs_per_month(Cadence::Manual), 0.0);
    }
}
