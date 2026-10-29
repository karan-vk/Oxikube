//! [`MutationGuard`]: the single pipeline every mutation passes (ADR 0012; E06-S02,
//! hardened in E19).
//!
//! ```text
//! dispatch ─> read-only check ─blocked─> Denied (ReadOnly error) ─────────────┐
//!                   │ allowed                                                 │
//!                   v                                                         v
//!             confirmation tier ─none yet─> [Irreversible: server dry run]  audit
//!                   │                        └─> NeedsConfirmation (no wait)  ^
//!                   │ confirmed / none needed                                 │
//!                   v                                                         │
//!             dry-run stage (was it dry-run?) ─> audit writable? ─no─> refused│
//!                   │                                                         │
//!                   v                                                         │
//!             handler with a Mutation ─> Succeeded / Failed ──────────────────┘
//! ```
//!
//! * **Read-only**: a session flagged read-only refuses every mutating command, for
//!   every [`Initiator`](oxikube_domain::audit::Initiator), with
//!   [`DispatchError::ReadOnly`](crate::command_bus::DispatchError::ReadOnly) naming
//!   the cluster. A cluster with no open session is refused too (fail closed).
//! * **Confirmation**: [`policy::confirm_tier_for`] derives the tier from the command's
//!   `CommandMeta`, its target (a Namespace, Node, PV or cascading delete is raised) and
//!   [`Risk`](oxikube_domain::safety::Risk). The guard answers
//!   [`Outcome::NeedsConfirmation`](crate::command_bus::Outcome::NeedsConfirmation)
//!   with a single-use [`ConfirmationToken`] and returns; the second dispatch carries the
//!   [`Confirmation`]. `TypeName` checks the typed name (exactly: no trimming, no case folding).
//!   [`CommandBus::execute`](crate::command_bus::CommandBus::execute) runs the same round trip
//!   through a [`ConfirmationProvider`] (an agent's permission prompt), routed by initiator with
//!   [`ConfirmationRouter`].
//! * **Dry run**: [`DispatchContext::dry_run`](crate::command_bus::DispatchContext::dry_run)
//!   reaches the handler through [`Mutation::write_options`]. A create, update, patch or apply
//!   ([`preview_verb`], E19-S07) and a command whose risk
//!   [requires it](oxikube_domain::safety::Risk::requires_dry_run_diff) (`Irreversible`) are
//!   dry-run by the guard (`dryRun=All`) before the confirmation is asked, and the request carries
//!   the result ([`DryRunPreview`]: each object written next to the live one, Secret values
//!   masked, and the server's warnings; or `NoPreviewAvailable` when the server cannot dry-run
//!   it, which never blocks); a refused dry run refuses the command, and a confirmation issued
//!   without the dry run cannot approve it. See the `rehearsal` and `preview` modules.
//! * **Execute**: the handler gets a [`Mutation`], the only way to a `ResourceWriter`.
//! * **Audit**: one [`AuditRecord`](oxikube_domain::audit::AuditRecord) per attempt
//!   (`Succeeded`, `Failed`, `Denied`, `Cancelled`) through [`AuditLog`]; a refusal says why
//!   ([`AuditDenial`](oxikube_domain::audit::AuditDenial): read-only, cancelled by the user, or
//!   policy), and every record of a bulk command shares its `batch_id`.
//!   Failing to audit fails the mutation closed (see [`crate::audit`]). The record is
//!   armed before the handler runs, so a dispatch future dropped mid-handler is still
//!   audited (as `Cancelled`).
//!
//! The checks before the handler are synchronous and in memory; a confirmation request
//! is returned before any `.await`.
//!
//! Commands of the *exec class* (`pod::Shell`, `pod::Attach`, `pod::Exec`: a session in a
//! container) are not mutations and take no confirmation, but they take the read-only block
//! (unless the cluster's `exec_in_read_only` setting allows them) and the audit record; see the
//! `exec` module of this one.

mod confirm;
mod decline;
mod exec;
mod gate;
mod mutation;
mod pipeline;
pub mod policy;
pub mod posture;
mod preview;
mod provider;
mod rehearsal;
mod selection_policy;
mod workload_policy;

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
pub use preview::{
    DryRunResult, MAX_PREVIEW_OBJECTS, PreviewChange, no_preview_reason, preview_verb,
};
pub use provider::{ConfirmationProvider, ConfirmationRouter, Decision};
pub use rehearsal::DryRunPreview;

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
    audit: Arc<AuditLog>,
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
            audit: Arc::new(AuditLog::new(state, clock)),
            confirmations: PendingConfirmations::default(),
        }
    }

    /// The audit log.
    pub fn audit(&self) -> &AuditLog {
        &self.audit
    }

    /// A shared handle on the audit log, for a service that must write a record after the guarded
    /// command returned (`ExecService`: the deletion of a node shell's pod when its terminal
    /// closes). Appends only; the guard stays the one place that decides what may run.
    pub fn audit_handle(&self) -> Arc<AuditLog> {
        self.audit.clone()
    }

    /// How many confirmation requests are waiting for an answer.
    pub fn pending_confirmations(&self) -> usize {
        self.confirmations.len()
    }
}
