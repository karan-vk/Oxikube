//! [`TransferProgress`] and the meter that reports it at most [`PROGRESS_INTERVAL`] apart.

use std::sync::Arc;
use std::time::{Duration, Instant};

/// How far a transfer got.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct TransferProgress {
    /// Bytes moved so far (tar stream bytes for a tar transfer, file bytes for `cat`).
    pub bytes: u64,
    /// The size the caller expects, when it knows one ([`TransferOptions::total_hint`]): a tar
    /// stream's length is unknown in advance, and is a little larger than the files in it.
    ///
    /// [`TransferOptions::total_hint`]: super::TransferOptions::total_hint
    pub total: Option<u64>,
}

impl TransferProgress {
    /// The fraction done in `0.0..=1.0`, when a total is known (and not zero). Clamped, since a
    /// tar stream overshoots the files' size by its headers.
    pub fn fraction(&self) -> Option<f64> {
        let total = self.total.filter(|total| *total > 0)?;
        #[expect(
            clippy::cast_precision_loss,
            reason = "a progress fraction does not need the low bits of a byte count"
        )]
        let fraction = self.bytes as f64 / total as f64;
        Some(fraction.min(1.0))
    }
}

/// Receives a transfer's progress. Called from the task that polls the transfer, so it must be
/// cheap and must not block: store the value and let the UI read it at frame cadence.
pub type ProgressFn = Arc<dyn Fn(TransferProgress) + Send + Sync>;

/// The shortest gap between two progress reports: at most ten a second, plus the final one.
pub const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

/// Counts a transfer's bytes and reports them through a [`ProgressFn`], throttled.
pub(super) struct ProgressMeter {
    report: Option<ProgressFn>,
    bytes: u64,
    total: Option<u64>,
    last: Option<Instant>,
    finished: bool,
}

impl ProgressMeter {
    pub(super) fn new(report: Option<ProgressFn>, total: Option<u64>) -> Self {
        Self {
            report,
            bytes: 0,
            total,
            last: None,
            finished: false,
        }
    }

    /// The bytes counted so far.
    pub(super) fn bytes(&self) -> u64 {
        self.bytes
    }

    fn progress(&self) -> TransferProgress {
        TransferProgress {
            bytes: self.bytes,
            total: self.total,
        }
    }

    /// Counts `n` more bytes, and reports if the last report is old enough.
    pub(super) fn advance(&mut self, n: usize) {
        if self.report.is_some() {
            self.advance_at(n, Instant::now());
        } else {
            self.bytes += n as u64;
        }
    }

    /// [`advance`](Self::advance) at the instant `now` (a test's clock).
    pub(super) fn advance_at(&mut self, n: usize, now: Instant) {
        self.bytes += n as u64;
        let Some(report) = &self.report else {
            return;
        };
        let due = self
            .last
            .is_none_or(|last| now.saturating_duration_since(last) >= PROGRESS_INTERVAL);
        if due {
            self.last = Some(now);
            report(self.progress());
        }
    }

    /// Reports the final count, once, whatever the throttle says.
    pub(super) fn finish(&mut self) {
        if std::mem::replace(&mut self.finished, true) {
            return;
        }
        if let Some(report) = &self.report {
            report(self.progress());
        }
    }
}
