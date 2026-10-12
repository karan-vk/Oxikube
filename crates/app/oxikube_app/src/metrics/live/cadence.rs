//! [`Cadence`]: when one kind of one cluster is polled next, from how its last polls went.
//!
//! | Last poll | Next poll after |
//! |---|---|
//! | samples | the interval (15 s by default) |
//! | the call failed, or the API answered 503 (`ApiUnavailable`) | twice the interval per failure in a row, capped at [`MAX_BACKOFF`] (15 s, 30 s, 60 s, 120 s, 120 s...) |
//! | not installed (404) or forbidden (403) | [`UNAVAILABLE_INTERVAL`]: a state that only an admin changes is asked about rarely |
//!
//! A poller that starts again after a pause (the table shown again, the tab brought back) waits
//! at most one interval since the last poll, whatever the backoff: whoever is looking gets a
//! fresh answer soon, and a hidden table never polls at all.

use std::time::Duration;

use jiff::Timestamp;
use oxikube_domain::OxiResult;
use oxikube_domain::metrics::MissingReason;
use oxikube_ports::MetricsOutcome;

/// The longest wait after failures in a row.
pub const MAX_BACKOFF: Duration = Duration::from_secs(120);
/// How often a cluster without the metrics API (or without the right to read it) is asked again.
pub const UNAVAILABLE_INTERVAL: Duration = Duration::from_secs(300);

/// The doubling stops after this many failures (2^3 = 8 intervals, before the cap).
const MAX_DOUBLINGS: u32 = 3;

/// The poll history of one kind of one cluster. Pure: the poller asks it when the next poll is
/// due and tells it how each poll went.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Cadence {
    /// When the last poll finished; `None` before the first.
    last: Option<Timestamp>,
    /// Failures in a row (call errors and 503s).
    failures: u32,
    /// The last answer was "not installed" or "forbidden".
    absent: bool,
    /// The poller restarted since the last poll: wait at most one interval.
    resumed: bool,
}

impl Cadence {
    /// How long to wait after the last poll, with polling interval `base`.
    pub fn delay(&self, base: Duration) -> Duration {
        let delay = if self.absent {
            UNAVAILABLE_INTERVAL.max(base)
        } else if self.failures == 0 {
            base
        } else {
            let doubled = base.saturating_mul(1 << self.failures.min(MAX_DOUBLINGS));
            doubled.min(MAX_BACKOFF.max(base))
        };
        if self.resumed { delay.min(base) } else { delay }
    }

    /// When the next poll is due: now for a kind never polled.
    pub fn due(&self, base: Duration) -> Option<Timestamp> {
        let last = self.last?;
        Some(last.saturating_add(self.delay(base)).unwrap_or(last))
    }

    /// Whether a poll is due at `now`.
    pub fn is_due(&self, base: Duration, now: Timestamp) -> bool {
        self.due(base).is_none_or(|due| due <= now)
    }

    /// Records a poll that finished at `at` with `result`.
    pub fn record(&mut self, at: Timestamp, result: &OxiResult<MetricsOutcome>) {
        self.last = Some(at);
        self.resumed = false;
        match result {
            Ok(MetricsOutcome::Available(_)) => {
                self.failures = 0;
                self.absent = false;
            }
            Ok(MetricsOutcome::Unavailable(
                MissingReason::NotInstalled | MissingReason::Forbidden,
            )) => {
                self.failures = 0;
                self.absent = true;
            }
            Ok(MetricsOutcome::Unavailable(_)) | Err(_) => {
                self.failures = self.failures.saturating_add(1);
                self.absent = false;
            }
        }
    }

    /// The poller starts again after a pause.
    pub fn resume(&mut self) {
        self.resumed = true;
    }

    /// Failures in a row so far.
    pub fn failures(&self) -> u32 {
        self.failures
    }
}
