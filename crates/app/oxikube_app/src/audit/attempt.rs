//! [`AuditAttempt`]: the armed record of a mutation whose handler is running.

use std::sync::{Arc, Mutex};

use oxikube_domain::audit::{AuditOutcome, AuditRecord};

use super::AuditLog;

/// Where a running handler leaves the words its record should carry
/// ([`HandlerContext::note_audit`](crate::command_bus::HandlerContext::note_audit)).
pub(crate) type NoteSlot = Arc<Mutex<Option<String>>>;

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
    /// Read when the attempt is dropped unfinished, so a cancelled record still names what the
    /// handler had noted by then (the batch id of a drain cancelled mid-way).
    note: Option<NoteSlot>,
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
            note: None,
        }
    }

    /// Makes a drop of the unfinished attempt add what `slot` holds then to the `Cancelled`
    /// record, as [`finish_noting`](Self::finish_noting) would.
    pub(crate) fn with_note_slot(mut self, slot: NoteSlot) -> Self {
        self.note = Some(slot);
        self
    }

    /// Queues the record with `outcome`, stamped now. Write it with
    /// [`AuditLog::flush`].
    pub fn finish(mut self, outcome: AuditOutcome) {
        if let Some(record) = self.record.take() {
            self.queue(record, outcome);
        }
    }

    /// [`finish`](Self::finish), adding `note` to the record's detail after what is there: what
    /// the handler learned by running (the Job a trigger created). Redacted like every detail.
    pub fn finish_noting(mut self, outcome: AuditOutcome, note: Option<&str>) {
        if let Some(record) = self.record.take() {
            self.queue(noted(record, note), outcome);
        }
    }

    fn queue(&self, mut record: AuditRecord, outcome: AuditOutcome) {
        record.outcome = outcome;
        self.log.stamp(&mut record);
        self.log.enqueue(record);
    }
}

/// `record` with `note` added after its detail.
fn noted(record: AuditRecord, note: Option<&str>) -> AuditRecord {
    let Some(note) = note else {
        return record;
    };
    let joined = match record.detail.as_deref() {
        Some(detail) => format!("{detail} {note}"),
        None => note.to_owned(),
    };
    AuditLog::describe(record, Some(&joined))
}

impl Drop for AuditAttempt<'_> {
    fn drop(&mut self) {
        if let Some(record) = self.record.take() {
            let note = self
                .note
                .as_ref()
                .and_then(|slot| slot.lock().ok().and_then(|mut slot| slot.take()));
            let record = noted(record, note.as_deref());
            tracing::warn!(
                command = %record.cmd,
                "mutation dropped before it finished; audited as cancelled"
            );
            self.queue(record, AuditOutcome::Cancelled);
        }
    }
}
