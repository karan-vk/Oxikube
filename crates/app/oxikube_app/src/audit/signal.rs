//! [`AuditAppended`] and [`AuditListener`]: the signal that the audit store gained records.

use tokio::sync::watch;

/// What an [`AuditListener`] sees after a flush stored records.
///
/// The signal only says that something was stored, and how many records so far; the viewer reads
/// the records themselves through its own query, so a burst of flushes costs it one read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AuditAppended {
    /// How many records this log has stored since the app started; grows with every flush.
    pub stored: u64,
}

impl AuditAppended {
    /// Counts `count` more stored records.
    pub(super) fn record_stored(&mut self, count: u64) {
        self.stored = self.stored.saturating_add(count);
    }
}

/// A listener for [`AuditAppended`] ([`AuditLog::appended`](super::AuditLog::appended)).
///
/// Changes coalesce: however many flushes happen between two looks, the listener wakes once and
/// sees the latest count. It runs on any executor (the viewer awaits it on GPUI's).
#[derive(Debug, Clone)]
pub struct AuditListener {
    rx: watch::Receiver<AuditAppended>,
}

impl AuditListener {
    pub(super) fn new(rx: watch::Receiver<AuditAppended>) -> Self {
        Self { rx }
    }

    /// The latest signal, without waiting.
    pub fn current(&self) -> AuditAppended {
        *self.rx.borrow()
    }

    /// Whether the log stored records since the last [`changed`](Self::changed).
    pub fn has_pending(&self) -> bool {
        self.rx.has_changed().unwrap_or(false)
    }

    /// Waits until the log stores records, then returns the latest signal; `None` when the log
    /// is gone.
    pub async fn changed(&mut self) -> Option<AuditAppended> {
        self.rx.changed().await.ok()?;
        Some(*self.rx.borrow_and_update())
    }
}
