//! The hot-path recorder: frame durations, feed deltas and `notify` counts.
//!
//! Besides the totals it keeps one assertion-style figure (E07-S09): the most coalesced notifies
//! delivered between two consecutive frames. Streams are coalesced to frame cadence, so with one
//! streaming view on screen it stays at 1 however fast the feed is; more means something notifies
//! outside `notify_coalesced`, or several streaming views redraw in the same frame.

use super::ring::{FrameRing, RingReader};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Frames the ring holds between flushes: about 34 s at 120 Hz, far more than one
/// [`super::DEFAULT_FLUSH_INTERVAL`].
pub const DEFAULT_FRAME_CAPACITY: usize = 4096;

/// Lock-free counters written by the UI thread (frames) and by any thread (feed, notify).
///
/// Every record call is a relaxed atomic add or a [`FrameRing::push`]: no lock, allocation or
/// I/O, so it is safe on the UI thread (non-negotiable 7).
pub struct Recorder {
    frames: FrameRing,
    feed_deltas: AtomicU64,
    notifies: AtomicU64,
    /// Notifies since the last recorded frame.
    notifies_since_frame: AtomicU64,
    /// The most notifies between two frames since the last drain.
    max_notifies_per_frame: AtomicU64,
}

impl Default for Recorder {
    fn default() -> Self {
        Self::new()
    }
}

impl Recorder {
    /// A recorder with [`DEFAULT_FRAME_CAPACITY`].
    pub fn new() -> Self {
        Self::with_frame_capacity(DEFAULT_FRAME_CAPACITY)
    }

    /// A recorder whose frame ring holds `capacity` frames between drains.
    pub fn with_frame_capacity(capacity: usize) -> Self {
        Self {
            frames: FrameRing::with_capacity(capacity),
            feed_deltas: AtomicU64::new(0),
            notifies: AtomicU64::new(0),
            notifies_since_frame: AtomicU64::new(0),
            max_notifies_per_frame: AtomicU64::new(0),
        }
    }

    /// Records one frame's duration. UI thread only (single producer). Closes the frame's notify
    /// count (see the module docs).
    #[inline]
    pub fn record_frame(&self, duration: Duration) {
        self.frames
            .push(u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX));
        let notifies = self.notifies_since_frame.swap(0, Ordering::Relaxed);
        self.max_notifies_per_frame
            .fetch_max(notifies, Ordering::Relaxed);
    }

    /// Adds `n` applied feed deltas.
    #[inline]
    pub fn record_feed_deltas(&self, n: u64) {
        self.feed_deltas.fetch_add(n, Ordering::Relaxed);
    }

    /// Counts one coalesced `notify`.
    #[inline]
    pub fn record_notify(&self) {
        self.notifies.fetch_add(1, Ordering::Relaxed);
        self.notifies_since_frame.fetch_add(1, Ordering::Relaxed);
    }

    /// Total frames recorded so far.
    pub fn frames_recorded(&self) -> u64 {
        self.frames.written()
    }

    /// Total feed deltas so far.
    pub fn feed_deltas(&self) -> u64 {
        self.feed_deltas.load(Ordering::Relaxed)
    }

    /// Total notifies so far.
    pub fn notifies(&self) -> u64 {
        self.notifies.load(Ordering::Relaxed)
    }

    /// A reader that sees everything recorded from the start.
    pub fn reader(&self) -> RecorderReader {
        RecorderReader {
            frames: RingReader::new(),
            feed_seen: 0,
            notify_seen: 0,
            last_drain: Instant::now(),
        }
    }
}

/// Everything recorded between two drains.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Tick {
    /// Frame durations, nanoseconds, in recording order.
    pub frames_ns: Vec<u64>,
    /// Frames lost because the reader fell a full ring behind.
    pub dropped_frames: u64,
    /// Feed deltas applied in the interval.
    pub feed_deltas: u64,
    /// Notifies in the interval.
    pub notifies: u64,
    /// The most notifies delivered between two consecutive frames in the interval (see the module
    /// docs).
    pub max_notifies_per_frame: u64,
    /// Wall time covered by this tick.
    pub interval: Duration,
}

/// Incremental reader of a [`Recorder`] (owned by the flush thread or a scenario driver).
pub struct RecorderReader {
    frames: RingReader,
    feed_seen: u64,
    notify_seen: u64,
    last_drain: Instant,
}

impl RecorderReader {
    /// Takes everything recorded since the previous drain.
    pub fn drain(&mut self, recorder: &Recorder) -> Tick {
        let mut frames_ns = Vec::new();
        let dropped_frames = self.frames.drain_into(&recorder.frames, &mut frames_ns);
        let feed = recorder.feed_deltas();
        let notify = recorder.notifies();
        let now = Instant::now();
        let tick = Tick {
            frames_ns,
            dropped_frames,
            feed_deltas: feed - self.feed_seen,
            notifies: notify - self.notify_seen,
            max_notifies_per_frame: recorder.max_notifies_per_frame.swap(0, Ordering::Relaxed),
            interval: now - self.last_drain,
        };
        self.feed_seen = feed;
        self.notify_seen = notify;
        self.last_drain = now;
        tick
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drain_reports_deltas_since_last_drain() {
        let recorder = Recorder::with_frame_capacity(8);
        let mut reader = recorder.reader();
        recorder.record_frame(Duration::from_micros(1500));
        recorder.record_frame(Duration::from_micros(2500));
        recorder.record_feed_deltas(40);
        recorder.record_notify();
        let tick = reader.drain(&recorder);
        assert_eq!(
            tick.max_notifies_per_frame, 0,
            "a notify after the last frame belongs to the next one"
        );
        assert_eq!(tick.frames_ns, [1_500_000, 2_500_000]);
        assert_eq!(
            (tick.feed_deltas, tick.notifies, tick.dropped_frames),
            (40, 1, 0)
        );

        recorder.record_feed_deltas(2);
        let tick = reader.drain(&recorder);
        assert!(tick.frames_ns.is_empty());
        assert_eq!((tick.feed_deltas, tick.notifies), (2, 0));
        assert_eq!(recorder.feed_deltas(), 42);
        assert_eq!(recorder.frames_recorded(), 2);
    }

    #[test]
    fn counts_the_most_notifies_between_two_frames() {
        let recorder = Recorder::with_frame_capacity(8);
        let mut reader = recorder.reader();
        recorder.record_notify();
        recorder.record_frame(Duration::from_millis(1));
        for _ in 0..3 {
            recorder.record_notify();
        }
        recorder.record_frame(Duration::from_millis(1));
        recorder.record_frame(Duration::from_millis(1));
        assert_eq!(reader.drain(&recorder).max_notifies_per_frame, 3);
        recorder.record_notify();
        recorder.record_frame(Duration::from_millis(1));
        assert_eq!(
            reader.drain(&recorder).max_notifies_per_frame,
            1,
            "the maximum is per drain"
        );
    }
}
