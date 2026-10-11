//! Deterministic sample data for the gallery, the tests and the perf scenario.

use crate::chart::{Point, Series};

/// A small deterministic noise source (an LCG), so fixtures do not depend on a RNG crate and
/// screenshots are stable.
struct Noise(u64);

impl Noise {
    fn next(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 33) as f64) / f64::from(1u32 << 31) - 0.5
    }
}

/// `count` samples ending at `now`, `step` seconds apart: `base` plus a sine of `amplitude` and
/// `period` samples plus a little noise, never below zero.
pub fn wave(
    name: &str,
    now: f64,
    count: usize,
    step: f64,
    base: f64,
    amplitude: f64,
    period: f64,
    seed: u64,
) -> Series {
    let mut noise = Noise(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let points: Vec<Point> = (0..count)
        .map(|i| {
            let t = now - step * (count - 1 - i) as f64;
            let phase = (i as f64 / period) * std::f64::consts::TAU;
            let v = base + amplitude * phase.sin() + amplitude * 0.15 * noise.next();
            Point::new(t, v.max(0.0))
        })
        .collect();
    Series::new(name.to_owned(), points)
}

/// `series` with a gap (a missed scrape) of three samples every `every` samples.
pub fn with_gaps(series: Series, every: usize) -> Series {
    let points: Vec<Point> = series
        .points
        .iter()
        .enumerate()
        .map(|(i, p)| {
            if i % every < 3 && i >= every {
                Point::gap(p.t)
            } else {
                *p
            }
        })
        .collect();
    Series::new(series.name, points).style(series.style)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waves_are_deterministic_ordered_and_end_at_now() {
        let a = wave("x", 1000.0, 100, 15.0, 5.0, 2.0, 20.0, 3);
        let b = wave("x", 1000.0, 100, 15.0, 5.0, 2.0, 20.0, 3);
        assert_eq!(a.points, b.points);
        assert_eq!(a.points.last().map(|p| p.t), Some(1000.0));
        assert!(a.points.windows(2).all(|w| w[0].t < w[1].t));
        assert!(a.points.iter().all(|p| p.v >= 0.0));
    }

    #[test]
    fn gaps_are_inserted_periodically() {
        let s = with_gaps(wave("x", 1000.0, 100, 15.0, 5.0, 2.0, 20.0, 3), 25);
        let gaps = s.points.iter().filter(|p| p.is_gap()).count();
        assert_eq!(gaps, 9);
        assert!(!s.points[0].is_gap());
    }
}
