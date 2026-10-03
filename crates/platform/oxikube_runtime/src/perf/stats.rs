//! Percentiles and the p50/p95/p99/max summary used by `--perf` and `cargo xtask perf`.

use serde::{Deserialize, Serialize};

/// Nearest-rank percentile of an ascending slice: the smallest value with at least `q` % of the
/// samples at or below it. `q` is clamped to `0..=100`; `None` for an empty slice.
///
/// Nearest-rank never interpolates, so on small samples p99 is the maximum (with 10 samples,
/// rank `ceil(0.99 * 10) = 10`), which is the conservative reading for frame budgets.
pub fn percentile_sorted(sorted: &[u64], q: f64) -> Option<u64> {
    if sorted.is_empty() {
        return None;
    }
    let q = q.clamp(0.0, 100.0);
    let rank = ((q / 100.0) * sorted.len() as f64).ceil() as usize;
    Some(sorted[rank.clamp(1, sorted.len()) - 1])
}

/// Distribution summary in milliseconds (three decimals).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Summary {
    /// Number of observations.
    pub count: u64,
    /// Median, ms.
    pub p50: f64,
    /// 95th percentile, ms.
    pub p95: f64,
    /// 99th percentile, ms.
    pub p99: f64,
    /// Largest observation, ms.
    pub max: f64,
}

impl Summary {
    /// Summarises nanosecond samples (sorts `samples` in place). `None` when empty.
    pub fn from_nanos(samples: &mut [u64]) -> Option<Self> {
        samples.sort_unstable();
        let ms = |q| percentile_sorted(samples, q).map(nanos_to_ms);
        Some(Self {
            count: samples.len() as u64,
            p50: ms(50.0)?,
            p95: ms(95.0)?,
            p99: ms(99.0)?,
            max: nanos_to_ms(*samples.last()?),
        })
    }

    /// A summary of one observation (all percentiles equal it).
    pub fn single(ms: f64) -> Self {
        let ms = round_ms(ms);
        Self {
            count: 1,
            p50: ms,
            p95: ms,
            p99: ms,
            max: ms,
        }
    }
}

/// Nanoseconds to milliseconds, rounded to microseconds.
pub fn nanos_to_ms(nanos: u64) -> f64 {
    round_ms(nanos as f64 / 1_000_000.0)
}

/// Rounds to three decimals (microsecond resolution) so reports stay readable.
pub fn round_ms(ms: f64) -> f64 {
    (ms * 1000.0).round() / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_has_no_percentile() {
        assert_eq!(percentile_sorted(&[], 50.0), None);
        assert_eq!(Summary::from_nanos(&mut []), None);
    }

    #[test]
    fn nearest_rank_on_hundred_values() {
        let v: Vec<u64> = (1..=100).collect();
        assert_eq!(percentile_sorted(&v, 50.0), Some(50));
        assert_eq!(percentile_sorted(&v, 95.0), Some(95));
        assert_eq!(percentile_sorted(&v, 99.0), Some(99));
        assert_eq!(percentile_sorted(&v, 100.0), Some(100));
        assert_eq!(percentile_sorted(&v, 0.0), Some(1));
    }

    #[test]
    fn p99_on_small_sample_is_the_max() {
        let v = [1, 2, 3, 4, 5, 6, 7, 8, 9, 50];
        assert_eq!(percentile_sorted(&v, 99.0), Some(50));
        assert_eq!(percentile_sorted(&v, 95.0), Some(50));
        assert_eq!(percentile_sorted(&v, 50.0), Some(5));
        assert_eq!(percentile_sorted(&[7], 99.0), Some(7));
        assert_eq!(percentile_sorted(&[3, 9], 50.0), Some(3));
    }

    #[test]
    fn summary_sorts_and_converts_to_ms() {
        // 10 frames: 1..=9 ms plus one 40 ms hitch, unsorted.
        let mut ns: Vec<u64> = vec![40, 3, 1, 2, 9, 8, 7, 6, 5, 4]
            .into_iter()
            .map(|ms| ms * 1_000_000)
            .collect();
        let s = Summary::from_nanos(&mut ns).unwrap();
        assert_eq!(s.count, 10);
        assert_eq!(s.p50, 5.0);
        assert_eq!(s.p95, 40.0);
        assert_eq!(s.p99, 40.0);
        assert_eq!(s.max, 40.0);
    }

    #[test]
    fn ms_rounding_keeps_microseconds() {
        assert_eq!(nanos_to_ms(1_234_567), 1.235);
        assert_eq!(Summary::single(12.34567).p99, 12.346);
    }
}
