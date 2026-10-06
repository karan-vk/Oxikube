//! [`AuditLog`]: batched, fail-closed appends of [`AuditRecord`]s through `StatePort`.

use std::collections::VecDeque;
use std::sync::Arc;

use futures::lock::Mutex;
use oxikube_domain::OxiResult;
use oxikube_domain::audit::{AuditOutcome, AuditRecord, Initiator};
use oxikube_domain::ids::ResourceRef;
use oxikube_domain::redact::redact;
use oxikube_ports::{ClockPort, StatePort};

/// Most records kept in memory while the store refuses writes. When the backlog is
/// full the oldest record is dropped (and logged); mutations are refused all that time,
/// so only denied and cancelled attempts can pile up.
pub const MAX_AUDIT_BACKLOG: usize = 1024;

/// The writer of the audit trail. See the [module docs](super).
///
/// Cheap to share behind an `Arc`; writes are serialised by an async lock that is held
/// across the `append_audit` call, so a flush that is in flight is never mistaken for a
/// healthy log by a concurrent mutation.
pub struct AuditLog {
    state: Arc<dyn StatePort>,
    clock: Arc<dyn ClockPort>,
    backlog: Mutex<VecDeque<AuditRecord>>,
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

    /// Appends `record` (after any backlog, in order).
    ///
    /// # Errors
    ///
    /// The store's error when the append fails; the record (and the backlog) is kept
    /// and retried by the next [`record`](Self::record) or
    /// [`ensure_writable`](Self::ensure_writable).
    pub async fn record(&self, record: AuditRecord) -> OxiResult<()> {
        let mut backlog = self.backlog.lock().await;
        backlog.push_back(record);
        self.flush_locked(&mut backlog).await
    }

    /// Flushes the backlog. `Ok` means the log is writable and a mutation may run.
    ///
    /// # Errors
    ///
    /// The store's error when the backlog could not be written; the caller must not
    /// mutate (fail closed).
    pub async fn ensure_writable(&self) -> OxiResult<()> {
        let mut backlog = self.backlog.lock().await;
        self.flush_locked(&mut backlog).await
    }

    /// How many records are waiting for the store to accept writes again (a snapshot for
    /// status display and tests; reads `0` while a write is in flight).
    pub fn backlog_len(&self) -> usize {
        self.backlog.try_lock().map_or(0, |b| b.len())
    }

    async fn flush_locked(&self, backlog: &mut VecDeque<AuditRecord>) -> OxiResult<()> {
        if backlog.is_empty() {
            return Ok(());
        }
        let batch = backlog.make_contiguous();
        match self.state.append_audit(batch).await {
            Ok(()) => {
                backlog.clear();
                Ok(())
            }
            Err(err) => {
                let excess = backlog.len().saturating_sub(MAX_AUDIT_BACKLOG);
                if excess > 0 {
                    backlog.drain(..excess);
                    tracing::error!(
                        dropped = excess,
                        "audit backlog full; oldest records dropped"
                    );
                }
                tracing::warn!(backlog = backlog.len(), error = %err, "audit append failed");
                Err(err)
            }
        }
    }
}
