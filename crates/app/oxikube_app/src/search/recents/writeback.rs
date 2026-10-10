//! `Writeback`: what the persisted lists share. A dirty flag, a wake-up for the writer task and a
//! "log the first failure only" latch.

use std::future::Future;
#[cfg(test)]
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tokio::sync::Notify;

/// How long the writer waits after the first change before it writes, so typing a few commands in
/// a row is one write.
pub const DEBOUNCE: Duration = Duration::from_millis(500);

/// Change tracking for one persisted value (or family of values).
#[derive(Debug, Default)]
pub(super) struct Writeback {
    /// Something changed since the last successful write.
    dirty: AtomicBool,
    /// Wakes the writer task; keeps one permit, so a change before the task waits is not lost.
    changed: Notify,
    /// A failure has been logged; the next ones are not, until a write succeeds again.
    warned: AtomicBool,
    /// How many failure lines were logged (the tests assert "once").
    #[cfg(test)]
    logged: AtomicUsize,
}

impl Writeback {
    /// Notes a change and wakes the writer.
    pub(super) fn mark(&self) {
        self.dirty.store(true, Ordering::Release);
        self.changed.notify_one();
    }

    /// Takes the dirty flag: `true` when there is something to write.
    pub(super) fn take(&self) -> bool {
        self.dirty.swap(false, Ordering::AcqRel)
    }

    /// A write failed: keep the change for the next attempt.
    pub(super) fn retry_later(&self) {
        self.dirty.store(true, Ordering::Release);
    }

    /// Whether there is something to write.
    pub(super) fn is_dirty(&self) -> bool {
        self.dirty.load(Ordering::Acquire)
    }

    /// Waits for a change, then for `pause(`[`DEBOUNCE`]`)`, so a burst of changes is one write.
    pub(super) async fn settled<P, F>(&self, pause: &P)
    where
        P: Fn(Duration) -> F,
        F: Future<Output = ()>,
    {
        self.changed.notified().await;
        pause(DEBOUNCE).await;
    }

    /// A write succeeded: the next failure is worth a line again.
    pub(super) fn healthy(&self) {
        self.warned.store(false, Ordering::Release);
    }

    /// Logs that `what` failed, once until the store works again. `cause` is the error's kind or
    /// a short description, never its message: the message of a storage error can carry a path,
    /// and nothing here is worth more than that.
    pub(super) fn failed(&self, what: &'static str, cause: impl std::fmt::Debug) {
        if !self.warned.swap(true, Ordering::AcqRel) {
            #[cfg(test)]
            self.logged.fetch_add(1, Ordering::AcqRel);
            tracing::warn!(?cause, "{what}; keeping it in memory");
        }
    }

    /// How many failure lines were logged.
    #[cfg(test)]
    pub(super) fn logged(&self) -> usize {
        self.logged.load(Ordering::Acquire)
    }
}
