//! [`MutationGuard`]: the single pipeline every mutation passes (ADR 0012; E06-S02,
//! hardened in E19).
//!
//! ```text
//! dispatch ─> read-only check ─blocked─> Denied (ReadOnly error) ─────────────┐
//!                   │ allowed                                                 │
//!                   v                                                         v
//!             confirmation tier ─none yet─> NeedsConfirmation (no wait)     audit
//!                   │ confirmed / none needed                                 ^
//!                   v                                                         │
//!             dry-run stage (stub, E19) ─> audit writable? ─no─> refused      │
//!                   │                                                         │
//!                   v                                                         │
//!             handler with a Mutation ─> Succeeded / Failed ──────────────────┘
//! ```
//!
//! * **Read-only**: a session flagged read-only refuses every mutating command, for
//!   every [`Initiator`](oxikube_domain::audit::Initiator), with
//!   [`DispatchError::ReadOnly`](crate::command_bus::DispatchError::ReadOnly) naming
//!   the cluster. A cluster with no open session is refused too (fail closed).
//! * **Confirmation**: [`policy::confirm_tier`] derives the tier from the command's
//!   `CommandMeta` and
//!   [`Risk`](oxikube_domain::safety::Risk). The guard answers
//!   [`Outcome::NeedsConfirmation`](crate::command_bus::Outcome::NeedsConfirmation)
//!   with a single-use [`ConfirmationToken`] and returns; the second dispatch carries the
//!   [`Confirmation`]. `TypeName` checks the typed name.
//! * **Dry run**: [`DispatchContext::dry_run`](crate::command_bus::DispatchContext::dry_run)
//!   reaches the handler through [`Mutation::write_options`]; the mandatory dry-run diff
//!   for `Irreversible` commands is E19.
//! * **Execute**: the handler gets a [`Mutation`], the only way to a `ResourceWriter`.
//! * **Audit**: one [`AuditRecord`](oxikube_domain::audit::AuditRecord) per attempt
//!   (`Succeeded`, `Failed`, `Denied`, `Cancelled`) through [`AuditLog`].
//!   Failing to audit fails the mutation closed (see [`crate::audit`]). The record is
//!   armed before the handler runs, so a dispatch future dropped mid-handler is still
//!   audited (as `Cancelled`).
//!
//! The checks before the handler are synchronous and in memory; a confirmation request
//! is returned before any `.await`.

mod confirm;
mod gate;
mod mutation;
mod pipeline;
pub mod policy;
pub mod posture;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use oxikube_ports::{ClockPort, StatePort};

pub use confirm::{
    Confirmation, ConfirmationError, ConfirmationRequest, ConfirmationToken,
    MAX_PENDING_CONFIRMATIONS,
};
pub use mutation::Mutation;
pub use posture::{PrefsPatch, PrefsWriter, register_commands};

use crate::audit::AuditLog;
use crate::session::ClusterSessionManager;
use confirm::PendingConfirmations;

/// Applies read-only mode, the confirmation policy, the dry-run stage and the audit
/// record to every mutating command. See the [module docs](self).
///
/// Built once and moved into the [`CommandBus`](crate::command_bus::CommandBus), which
/// is the only caller of the pipeline.
#[derive(Debug)]
pub struct MutationGuard {
    sessions: ClusterSessionManager,
    audit: AuditLog,
    confirmations: PendingConfirmations,
}

impl MutationGuard {
    /// A guard reading the read-only flag and the writer from `sessions`, appending
    /// audit records to `state`, stamped with `clock`.
    pub fn new(
        sessions: ClusterSessionManager,
        state: Arc<dyn StatePort>,
        clock: Arc<dyn ClockPort>,
    ) -> Self {
        Self {
            sessions,
            audit: AuditLog::new(state, clock),
            confirmations: PendingConfirmations::default(),
        }
    }

    /// The audit log.
    pub fn audit(&self) -> &AuditLog {
        &self.audit
    }

    /// How many confirmation requests are waiting for an answer.
    pub fn pending_confirmations(&self) -> usize {
        self.confirmations.len()
    }
}
