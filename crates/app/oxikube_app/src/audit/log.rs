//! [`AuditLog`]: batched, fail-closed appends of [`AuditRecord`]s through `StatePort`.

use std::collections::VecDeque;
use std::future::Future;
use std::sync::Arc;

use oxikube_domain::OxiResult;
use oxikube_domain::audit::{AuditOutcome, AuditRecord, Initiator};
use oxikube_domain::ids::ResourceRef;
use oxikube_domain::redact::redact;
use oxikube_ports::{ClockPort, StatePort};
use parking_lot::Mutex;

use super::attempt::AuditAttempt;

/// Most records kept in memory while the store refuses writes.
///
/// Mutations are refused all that time, so what piles up is mostly `Denied` records;
/// when the backlog is full the oldest `Denied` records are dropped first (and logged).
/// Records of mutations that may have reached the cluster (`Succeeded`, `Failed`,
/// `Cancelled`), such as the one whose own flush failed, are dropped oldest first only
/// when nothing else is left to drop.
pub const MAX_AUDIT_BACKLOG: usize = 1024;

/// The writer of the audit trail. See the [module docs](super).
///
/// Cheap to share behind an `Arc`. Records are queued in a synchronous backlog the
/// moment they are handed over (so a record survives its caller being cancelled), and
/// appends are serialised by an async lock held across the `append_audit` call, so a
/// flush that is in flight is never mistaken for a healthy log by a concurrent mutation.
pub struct AuditLog {
    state: Arc<dyn StatePort>,
    clock: Arc<dyn ClockPort>,
    /// Records accepted but not yet stored, oldest first. Never held across an
    /// `.await`, so `Drop` code can queue into it.
    backlog: Mutex<VecDeque<AuditRecord>>,
    /// Serialises appends; held across `append_audit`.
    writer: futures::lock::Mutex<()>,
}

impl std::fmt::Debug for AuditLog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuditLog").finish_non_exhaustive()
    }
}

impl AuditLog {
    /// A log that appends to `state` and stamps records with `clock`.
    pub fn new(state: Arc<dyn StatePort>, clock: Arc<dyn ClockPort>) -> Self {
        Self {
            state,
            clock,
            backlog: Mutex::new(VecDeque::new()),
            writer: futures::lock::Mutex::new(()),
        }
    }

    /// Builds a record stamped now. `who` is redacted; `cmd` is a command id.
    pub fn entry(
        &self,
        who: &str,
        initiator: Initiator,
        cmd: &str,
        target: ResourceRef,
        dry_run: bool,
        outcome: AuditOutcome,
    ) -> AuditRecord {
        AuditRecord::new(
            self.clock.now(),
            &redact(who),
            initiator,
            cmd,
            target,
            dry_run,
            outcome,
        )
    }

    /// [`entry`](Self::entry) with a [`detail`](AuditRecord::detail) (redacted like `who`): the
    /// container and program of a session opened in a pod.
    pub fn entry_with_detail(
        &self,
        who: &str,
        initiator: Initiator,
        cmd: &str,
        target: ResourceRef,
        detail: &str,
        outcome: AuditOutcome,
    ) -> AuditRecord {
        self.entry(who, initiator, cmd, target, false, outcome)
            .with_detail(&redact(detail))
    }

    /// `record` with `detail` (redacted like `who`) when there is one: the way a guarded mutation
    /// adds what a plain record does not say (the image of a debug container).
    pub fn describe(record: AuditRecord, detail: Option<&str>) -> AuditRecord {
        match detail {
            Some(detail) => record.with_detail(&redact(detail)),
            None => record,
        }
    }

    /// [`begin`](Self::begin) with a [`detail`](AuditRecord::detail) (redacted like `who`).
    pub fn begin_with_detail(
        &self,
        who: &str,
        initiator: Initiator,
        cmd: &str,
        target: ResourceRef,
        detail: &str,
    ) -> AuditAttempt<'_> {
        self.begin_described(who, initiator, cmd, target, false, Some(detail))
    }

    /// [`begin`](Self::begin) with an optional [`detail`](AuditRecord::detail): the one the other
    /// `begin`s are built on.
    pub fn begin_described(
        &self,
        who: &str,
        initiator: Initiator,
        cmd: &str,
        target: ResourceRef,
        dry_run: bool,
        detail: Option<&str>,
    ) -> AuditAttempt<'_> {
        let record = self.entry(
            who,
            initiator,
            cmd,
            target,
            dry_run,
            AuditOutcome::Cancelled,
        );
        AuditAttempt::new(self, Self::describe(record, detail))
    }

    /// Opens the record of a mutation that is about to run. The returned
    /// [`AuditAttempt`] queues its final record when it is
    /// [finished](AuditAttempt::finish), or a `Cancelled` record when it is dropped
    /// first (the caller's future was cancelled while the mutation was in flight).
    /// Either way the record is in the backlog without any `.await`; write it with
    /// [`flush`](Self::flush).
    pub fn begin(
        &self,
        who: &str,
        initiator: Initiator,
        cmd: &str,
        target: ResourceRef,
        dry_run: bool,
    ) -> AuditAttempt<'_> {
        self.begin_described(who, initiator, cmd, target, dry_run, None)
    }

    /// Appends `record` (after any backlog, in order).
    ///
    /// The record joins the backlog when this is called, before the returned future is
    /// first polled, so dropping the future never loses it: the next
    /// [`flush`](Self::flush) writes it.
    ///
    /// # Errors
    ///
    /// The store's error when the append fails; the record (and the backlog) is kept
    /// and retried by the next [`record`](Self::record) or [`flush`](Self::flush).
    pub fn record(&self, record: AuditRecord) -> impl Future<Output = OxiResult<()>> + '_ {
        self.enqueue(record);
        self.flush()
    }

    /// Flushes the backlog. `Ok` means the log is writable and a mutation may run.
    ///
    /// # Errors
    ///
    /// The store's error when the backlog could not be written; the caller must not
    /// mutate (fail closed).
    pub async fn ensure_writable(&self) -> OxiResult<()> {
        self.flush().await
    }

    /// Writes every queued record in one batch.
    ///
    /// # Errors
    ///
    /// The store's error; the records stay queued (up to [`MAX_AUDIT_BACKLOG`]).
    pub async fn flush(&self) -> OxiResult<()> {
        let _writer = self.writer.lock().await;
        let batch: Vec<AuditRecord> = self.backlog.lock().iter().cloned().collect();
        if batch.is_empty() {
            return Ok(());
        }
        match self.state.append_audit(&batch).await {
            Ok(()) => {
                // Only the holder of `writer` removes records, and new ones are pushed
                // at the back, so the batch is still the front of the queue.
                self.backlog.lock().drain(..batch.len());
                Ok(())
            }
            Err(err) => {
                let mut backlog = self.backlog.lock();
                trim_backlog(&mut backlog);
                tracing::warn!(backlog = backlog.len(), error = %err, "audit append failed");
                Err(err)
            }
        }
    }

    /// How many records are waiting to be written (a snapshot for status display and
    /// tests; includes a batch whose write is in flight).
    pub fn backlog_len(&self) -> usize {
        self.backlog.lock().len()
    }

    /// Queues `record` behind the backlog without writing it. Synchronous, so it is
    /// safe from `Drop`.
    pub(super) fn enqueue(&self, record: AuditRecord) {
        self.backlog.lock().push_back(record);
    }

    /// Stamps `record` with the current time (the moment the attempt ended).
    pub(super) fn stamp(&self, record: &mut AuditRecord) {
        record.ts = self.clock.now();
    }
}

/// Cuts `backlog` down to [`MAX_AUDIT_BACKLOG`]: the oldest `Denied` records go first,
/// then (last resort) the oldest of the rest. Logs what it dropped.
fn trim_backlog(backlog: &mut VecDeque<AuditRecord>) {
    let excess = backlog.len().saturating_sub(MAX_AUDIT_BACKLOG);
    if excess == 0 {
        return;
    }
    let mut denied = 0;
    backlog.retain(|record| {
        let drop = denied < excess && record.outcome == AuditOutcome::Denied;
        denied += usize::from(drop);
        !drop
    });
    let mutations = excess - denied;
    backlog.drain(..mutations);
    if denied > 0 {
        tracing::error!(
            dropped = denied,
            "audit backlog full; oldest denied records dropped"
        );
    }
    if mutations > 0 {
        tracing::error!(
            dropped = mutations,
            "audit backlog full of mutation records; oldest dropped"
        );
    }
}
