//! [`AuditAttempt`]: the armed record of a mutation whose handler is running.

use oxikube_domain::audit::{AuditOutcome, AuditRecord};

use super::AuditLog;

/// The record of one mutation attempt while its handler runs, from
/// [`AuditLog::begin`].
///
/// A handler runs on whatever task awaits `CommandBus::dispatch`, and that future may
/// be dropped mid-handler (a timed-out agent request, a closed view) after the write
/// already reached the API server. So the attempt is armed before the handler starts:
///
/// * [`finish`](Self::finish) queues the record with the handler's outcome;
/// * dropping it unfinished queues it as
///   [`Cancelled`](AuditOutcome::Cancelled) (the outcome is unknown, the request may
///   have been applied) and logs a warning.
///
/// Both paths queue synchronously, so the record is never lost; the next
/// [`AuditLog::flush`] (which every later mutation does first) writes it, and the guard
/// stays fail closed if it cannot.
#[must_use = "dropping an attempt records it as cancelled"]
pub struct AuditAttempt<'a> {
    log: &'a AuditLog,
    record: Option<AuditRecord>,
}

impl std::fmt::Debug for AuditAttempt<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuditAttempt")
            .field("record", &self.record)
            .finish_non_exhaustive()
    }
}

impl<'a> AuditAttempt<'a> {
    pub(super) fn new(log: &'a AuditLog, record: AuditRecord) -> Self {
        Self {
            log,
            record: Some(record),
        }
    }

    /// Queues the record with `outcome`, stamped now. Write it with
    /// [`AuditLog::flush`].
    pub fn finish(mut self, outcome: AuditOutcome) {
        if let Some(record) = self.record.take() {
            self.queue(record, outcome);
        }
    }

    fn queue(&self, mut record: AuditRecord, outcome: AuditOutcome) {
        record.outcome = outcome;
        self.log.stamp(&mut record);
        self.log.enqueue(record);
    }
}

impl Drop for AuditAttempt<'_> {
    fn drop(&mut self) {
        if let Some(record) = self.record.take() {
            tracing::warn!(
                command = %record.cmd,
                "mutation dropped before it finished; audited as cancelled"
            );
            self.queue(record, AuditOutcome::Cancelled);
        }
    }
}
