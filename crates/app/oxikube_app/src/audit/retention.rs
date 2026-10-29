//! [`AuditRetention`]: prunes audit records older than `audit.retention_days`, on start and daily.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use futures::channel::mpsc;
use futures::{FutureExt, StreamExt, select};
use jiff::{SignedDuration, Timestamp};
use oxikube_domain::OxiResult;
use oxikube_ports::{ClockPort, StatePort};

/// The longest `audit.retention_days` (ten years); larger values are clamped.
pub const MAX_RETENTION_DAYS: u32 = 3650;

/// How long the scheduler waits between prunes.
pub const PRUNE_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// Keeps the audit log within its retention window.
///
/// [`run`](Self::run) prunes once when it starts and then every [`PRUNE_INTERVAL`] on the clock it
/// was given; [`set_days`](Self::set_days) changes the window and prunes at once (the setting is
/// hot-reloaded). `0` days keeps everything. The actual delete and the vacuum run on the state
/// store's own thread (`StatePort::prune_audit`), never on the caller's.
pub struct AuditRetention {
    state: Arc<dyn StatePort>,
    clock: Arc<dyn ClockPort>,
    days: AtomicU32,
    wake: mpsc::UnboundedSender<()>,
    woken: futures::lock::Mutex<mpsc::UnboundedReceiver<()>>,
}

impl std::fmt::Debug for AuditRetention {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuditRetention")
            .field("days", &self.days())
            .finish_non_exhaustive()
    }
}

impl AuditRetention {
    /// A scheduler pruning `state` to `days` days (`0` keeps everything), timed by `clock`.
    pub fn new(state: Arc<dyn StatePort>, clock: Arc<dyn ClockPort>, days: u32) -> Self {
        let (wake, woken) = mpsc::unbounded();
        Self {
            state,
            clock,
            days: AtomicU32::new(days.min(MAX_RETENTION_DAYS)),
            wake,
            woken: futures::lock::Mutex::new(woken),
        }
    }

    /// The current window in days.
    pub fn days(&self) -> u32 {
        self.days.load(Ordering::Relaxed)
    }

    /// Changes the window (clamped to [`MAX_RETENTION_DAYS`]) and asks the running scheduler to
    /// prune now. Does nothing when the value did not change.
    pub fn set_days(&self, days: u32) {
        let days = days.min(MAX_RETENTION_DAYS);
        if self.days.swap(days, Ordering::Relaxed) != days {
            // A closed channel means the scheduler is gone; the next `run` prunes anyway.
            let _ = self.wake.unbounded_send(());
        }
    }

    /// Removes the records older than the window. Returns how many went (`0` when the window is
    /// `0` days, which keeps everything).
    ///
    /// # Errors
    ///
    /// The store's error.
    pub async fn prune_now(&self) -> OxiResult<u64> {
        let days = self.days();
        if days == 0 {
            return Ok(0);
        }
        let window = SignedDuration::from_hours(i64::from(days) * 24);
        let cutoff = self
            .clock
            .now()
            .checked_sub(window)
            .unwrap_or(Timestamp::MIN);
        self.state.prune_audit(cutoff).await
    }

    /// Prunes now, then once a day and whenever [`set_days`](Self::set_days) changes the window,
    /// for as long as the future is polled. A failed prune is logged and retried at the next
    /// tick. Hold the task that runs it; dropping the task stops it.
    pub async fn run(&self) {
        let mut woken = self.woken.lock().await;
        loop {
            match self.prune_now().await {
                Ok(0) => {}
                Ok(removed) => tracing::info!(removed, days = self.days(), "audit log pruned"),
                Err(error) => tracing::warn!(%error, "audit retention prune failed"),
            }
            select! {
                () = self.clock.sleep(PRUNE_INTERVAL).fuse() => {}
                signal = woken.next() => {
                    if signal.is_none() {
                        return;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
