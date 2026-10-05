//! Tuning for [`KubeLogs`](super::KubeLogs).

use std::time::Duration;

/// Settings for one [`KubeLogs`](super::KubeLogs).
///
/// The defaults follow kdash's constants (`BATCH_SIZE`, `BATCH_FLUSH_MS`,
/// `RECONNECT_OVERLAP_SECS`, `MAX_RECONNECT_ATTEMPTS`) except where noted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogsConfig {
    /// Lines per batch handed to the consumer. A batch is one channel send, so a bigger
    /// batch means fewer wake-ups at 5 000 lines/s. Default 50 (kdash `BATCH_SIZE`).
    pub batch_size: usize,
    /// A partly filled batch is flushed after this long, which bounds the added latency of
    /// a quiet stream. Default 100 ms (kdash `BATCH_FLUSH_MS`).
    pub flush_interval: Duration,
    /// Batches buffered between the reader and the consumer. The reader stops reading from
    /// the socket when the buffer is full, so a slow consumer applies backpressure to the
    /// server instead of growing memory. Default 16.
    pub channel_batches: usize,
    /// How far before the last line seen a reconnect asks the server to start (kdash
    /// `RECONNECT_OVERLAP_SECS`). The server rounds `sinceTime` to whole seconds, so values
    /// below one second are raised to one. Default 5 s.
    pub reconnect_overlap: Duration,
    /// How many recent lines are remembered to drop the replayed overlap after a reconnect.
    /// It must cover `reconnect_overlap` times the line rate (5 000 lines/s over 5 s is
    /// 25 000); each entry is one 64-bit hash, so the default 65 536 costs about 1.5 MB per
    /// followed container at most. Lines beyond it could replay as duplicates.
    pub dedup_window: usize,
    /// First pause before a reconnect; doubles per unproductive cycle. Default 250 ms.
    pub backoff_initial: Duration,
    /// Upper bound of the reconnect pause. Default 5 s.
    pub backoff_max: Duration,
    /// Consecutive failed (re)opens, not explained by the container waiting to start, before
    /// the stream gives up with an error (kdash `MAX_RECONNECT_ATTEMPTS`). Default 10.
    pub max_open_failures: u32,
    /// Most containers a single fan-in stream follows at once (kdash caps at 20 pods).
    /// Further containers are skipped with a warning. Default 64.
    pub max_fanin_streams: usize,
}

impl Default for LogsConfig {
    fn default() -> Self {
        Self {
            batch_size: 50,
            flush_interval: Duration::from_millis(100),
            channel_batches: 16,
            reconnect_overlap: Duration::from_secs(5),
            dedup_window: 65_536,
            backoff_initial: Duration::from_millis(250),
            backoff_max: Duration::from_secs(5),
            max_open_failures: 10,
            max_fanin_streams: 64,
        }
    }
}

impl LogsConfig {
    /// The pause before reconnect number `idle_cycles` (0 for the first).
    pub(super) fn backoff(&self, idle_cycles: u32) -> Duration {
        let factor = 1u32.checked_shl(idle_cycles.min(16)).unwrap_or(u32::MAX);
        self.backoff_initial
            .saturating_mul(factor)
            .min(self.backoff_max)
    }

    /// `reconnect_overlap` as whole seconds, at least one (the server's `sinceTime` precision).
    pub(super) fn overlap_secs(&self) -> i64 {
        i64::try_from(self.reconnect_overlap.as_secs())
            .unwrap_or(i64::MAX)
            .max(1)
    }
}
