//! The mandatory dry run (ADR 0012): what the guard does before it asks the user to confirm a
//! command whose risk [requires a dry run](Risk::requires_dry_run_diff) (an `Irreversible`
//! delete of a Namespace or a PersistentVolume, or a bulk delete that holds one).
//!
//! ```text
//! first dispatch ─> admit ─> needs a dry run? ─no──> NeedsConfirmation (preview: None)
//!                                  │ yes
//!                                  v
//!                 the handler, as a server dry run (gate forces dry_run, audited dry_run=true)
//!                    │ ok                 │ Unsupported              │ any other error
//!                    v                    v                          v
//!            preview: Rehearsed    preview: NoPreviewAvailable    refused: the server's
//!                    └──────> NeedsConfirmation <──┘              error, nothing asked
//! ```
//!
//! The rehearsal is the command's own handler run once with a dry-run [`Mutation`], so the dry
//! run sends exactly the requests the real run will, and every handler that honours
//! [`Mutation::dry_run`] (they all do: a dry-run dispatch is the same path) previews itself. The
//! writer it gets forces `dry_run` on every request besides (`ReadOnlyGate::rehearsal`), so a
//! rehearsal never changes the cluster. The token issued afterwards remembers that the dry run
//! happened; a confirmation for a command that needed one and did not get one is refused
//! ([`DispatchError::DryRunRequired`]).
//!
//! A server that cannot dry-run the request (an aggregated API without dry-run support,
//! [`ErrorKind::Unsupported`]) never blocks the mutation: the request carries
//! [`DryRunPreview::NoPreviewAvailable`] and the dialog says so.

use std::sync::Arc;

use oxikube_domain::ErrorKind;
use oxikube_domain::audit::AuditOutcome;
use oxikube_domain::command::{Command, CommandMeta};
use oxikube_domain::safety::Risk;

use super::gate::ReadOnlyGate;
use super::pipeline::Admitted;
use super::{Mutation, MutationGuard};
use crate::command_bus::{
    CommandHandler, CommandOutput, DispatchContext, DispatchError, HandlerContext,
};

/// What the server-side dry run of a command showed, attached to its
/// [`ConfirmationRequest`](super::ConfirmationRequest).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DryRunPreview {
    /// The server accepted a dry run of exactly this command; what its handler reported (for a
    /// delete, `Dry run: namespace demo can be deleted`).
    Rehearsed(CommandOutput),
    /// The server (an aggregated API, an older cluster) does not support a dry run of this
    /// request. The mutation may still run once confirmed: a missing preview never blocks it.
    NoPreviewAvailable {
        /// The server's words, for the dialog's banner.
        reason: String,
    },
}

impl DryRunPreview {
    /// Whether the server could dry-run the command.
    pub fn is_available(&self) -> bool {
        matches!(self, DryRunPreview::Rehearsed(_))
    }
}

/// Whether `command`, dispatched with `ctx`, must be dry-run by the guard before it is confirmed:
/// its risk [requires it](Risk::requires_dry_run_diff) and the dispatch is not a dry run itself.
pub(super) fn needs_rehearsal(command: &Command, ctx: &DispatchContext) -> bool {
    !ctx.dry_run
        && command
            .effective_risk()
            .is_some_and(Risk::requires_dry_run_diff)
}

/// The dry-run stage of a confirmed dispatch: refuses a command that needed the guard's dry run
/// when the confirmation it carries was issued without one.
///
/// # Errors
///
/// [`DispatchError::DryRunRequired`] in that case.
pub(super) fn require_rehearsal(
    meta: &CommandMeta,
    command: &Command,
    ctx: &DispatchContext,
    rehearsed: bool,
) -> Result<(), DispatchError> {
    if needs_rehearsal(command, ctx) && !rehearsed {
        return Err(DispatchError::DryRunRequired(meta.id));
    }
    Ok(())
}

impl MutationGuard {
    /// Runs `handler` for `command` once as a server-side dry run and returns what it showed.
    ///
    /// The attempt is audited like any dispatch (`dry_run` set; `Succeeded`, or `Failed` when the
    /// server refused or could not dry-run it), and needs a writable log first (fail closed).
    ///
    /// # Errors
    ///
    /// [`DispatchError::Handler`] with the server's error when it refused the dry run (`Forbidden`,
    /// `NotFound`, an admission webhook): the real request would be refused too, so nothing is
    /// asked. [`DispatchError::AuditUnavailable`] / [`DispatchError::AuditFailed`] when the log
    /// cannot be written.
    pub(super) async fn rehearse(
        &self,
        meta: &CommandMeta,
        command: &Command,
        ctx: &DispatchContext,
        admitted: &Admitted,
        handler: Arc<dyn CommandHandler>,
    ) -> Result<DryRunPreview, DispatchError> {
        self.audit
            .ensure_writable()
            .await
            .map_err(DispatchError::AuditUnavailable)?;
        let attempt = self.audit.begin_described(
            &ctx.who,
            ctx.initiator,
            meta.id.as_str(),
            admitted.target.clone(),
            true,
            admitted.detail.as_deref(),
        );
        let writer = Arc::new(ReadOnlyGate::rehearsal(
            self.sessions.clone(),
            admitted.cluster.clone(),
            admitted.context.clone(),
            admitted.writer.clone(),
        ));
        let mutation = Mutation::new(admitted.cluster.clone(), writer, true);
        let cx = HandlerContext::new(
            ctx.initiator,
            ctx.who.clone(),
            Some(admitted.cluster.clone()),
            Some(mutation),
        )
        .with_audit(self.audit.clone());
        let note = cx.audit_note_slot();
        let attempt = attempt.with_note_slot(note.clone());
        let result = handler.handle(command.clone(), cx).await;

        let note = note.lock().ok().and_then(|mut slot| slot.take());
        attempt.finish_noting(
            if result.is_ok() {
                AuditOutcome::Succeeded
            } else {
                AuditOutcome::Failed
            },
            note.as_deref(),
        );
        self.audit
            .flush()
            .await
            .map_err(DispatchError::AuditFailed)?;
        match result {
            Ok(output) => Ok(DryRunPreview::Rehearsed(output)),
            Err(error) if error.kind() == ErrorKind::Unsupported => {
                tracing::info!(command = %meta.id, "no dry run available; asking without a preview");
                Ok(DryRunPreview::NoPreviewAvailable {
                    reason: error.message().to_owned(),
                })
            }
            Err(error) => Err(DispatchError::Handler(error)),
        }
    }
}
