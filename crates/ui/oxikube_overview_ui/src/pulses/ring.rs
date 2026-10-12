//! [`PulseRing`]: the last [`PULSE_SAMPLES`] readings of one tile, for its sparkline.
//!
//! A fixed-size ring made once per tile. A push overwrites the oldest reading when full, so the
//! memory of a page that stays open for days does not move. [`points`](PulseRing::points) hands
//! the sparkline an `Arc` it can compare by pointer: the same `Arc` while nothing was pushed,
//! so the sparkline keeps its tessellated path and a repaint allocates nothing.

use std::sync::Arc;

use oxikube_ui::chart::Point;

/// Readings kept per tile: 60, which is five minutes at [`HISTORY_INTERVAL`](super::HISTORY_INTERVAL).
pub const PULSE_SAMPLES: usize = 60;

/// The recent readings of one tile, oldest first.
#[derive(Debug)]
pub struct PulseRing {
    slots: Box<[Point; PULSE_SAMPLES]>,
    /// Where the next reading goes.
    head: usize,
    len: usize,
    /// The readings as the sparkline wants them; `None` after a push, rebuilt on demand.
    cached: Option<Arc<[Point]>>,
}

impl Default for PulseRing {
    fn default() -> Self {
        Self::new()
    }
}

impl PulseRing {
    /// An empty ring (its one allocation is made here).
    pub fn new() -> Self {
        Self {
            slots: Box::new([Point::new(0.0, 0.0); PULSE_SAMPLES]),
            head: 0,
            len: 0,
            cached: None,
        }
    }

    /// Adds a reading of `value` at `at` (seconds since the epoch), overwriting the oldest when
    /// full. A reading at or before the newest one's time replaces it instead (a clock that went
    /// back, or two reads in one second), so the series stays ascending in time.
    pub fn push(&mut self, at: f64, value: f64) {
        self.cached = None;
        if let Some(newest) = self.newest_slot()
            && at <= self.slots[newest].t
        {
            self.slots[newest] = Point::new(self.slots[newest].t, value);
            return;
        }
        self.slots[self.head] = Point::new(at, value);
        self.head = (self.head + 1) % PULSE_SAMPLES;
        self.len = (self.len + 1).min(PULSE_SAMPLES);
    }

    /// Replaces the whole ring with `readings` (oldest first; only the newest
    /// [`PULSE_SAMPLES`] are kept), for a history that arrives whole (the cluster gauges').
    pub fn replace(&mut self, readings: impl IntoIterator<Item = (f64, f64)>) {
        self.cached = None;
        self.head = 0;
        self.len = 0;
        for (at, value) in readings {
            self.push(at, value);
        }
    }

    /// How many readings it holds.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether it holds none.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The newest reading's value.
    pub fn latest(&self) -> Option<f64> {
        self.newest_slot().map(|slot| self.slots[slot].v)
    }

    /// The readings, oldest first.
    pub fn iter(&self) -> impl Iterator<Item = Point> + '_ {
        let start = (self.head + PULSE_SAMPLES - self.len) % PULSE_SAMPLES;
        (0..self.len).map(move |i| self.slots[(start + i) % PULSE_SAMPLES])
    }

    /// The readings for a sparkline: the same `Arc` until the next push.
    pub fn points(&mut self) -> Arc<[Point]> {
        if let Some(points) = &self.cached {
            return points.clone();
        }
        let points: Arc<[Point]> = self.iter().collect();
        self.cached = Some(points.clone());
        points
    }

    fn newest_slot(&self) -> Option<usize> {
        (self.len > 0).then(|| (self.head + PULSE_SAMPLES - 1) % PULSE_SAMPLES)
    }
}
