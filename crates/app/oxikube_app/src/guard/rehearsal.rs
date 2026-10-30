//! The guard's dry run (ADR 0012, E19-S02, E19-S07): what the guard does before it asks the user
//! to confirm a command it previews (a create, update, patch or apply, see `preview`) or whose
//! risk [requires a dry run](Risk::requires_dry_run_diff) (an `Irreversible` delete of a Namespace
//! or a PersistentVolume, or a bulk delete that holds one).
//!
//! ```text
//! first dispatch ─> admit ─> needs a dry run? ─no──> NeedsConfirmation (preview: None)
//!                                  │ yes
//!                                  v
//!                 the handler, as a server dry run (gate forces dry_run, audited dry_run=true)
//!                    │ ok                 │ cannot dry-run          │ any other error
//!                    v                    v                          v
//!            preview: Rehearsed    preview: NoPreviewAvailable    refused: the server's
//!            (diff, warnings)             │                       error, nothing asked
//!                    └──────> NeedsConfirmation <──┘
//! ```
//!
//! The rehearsal is the command's own handler run once with a dry-run [`Mutation`], so the dry
//! run sends exactly the requests the real run will, and every handler that honours
//! [`Mutation::dry_run`] (they all do: a dry-run dispatch is the same path) previews itself. The
//! writer it gets forces `dry_run` on every request besides (`ReadOnlyGate::rehearsal`), so a
//! rehearsal never changes the cluster, and keeps what the server answered for the diff. The token
//! issued afterwards remembers that the dry run happened; a confirmation for a command that needed
//! one and did not get one is refused ([`DispatchError::DryRunRequired`]).
//!
//! A server that cannot dry-run the request (an aggregated API without dry-run support, an
//! admission webhook with side effects, see [`no_preview_reason`]) never blocks the mutation: the
//! request carries [`DryRunPreview::NoPreviewAvailable`] and the dialog shows a banner. The token
//! remembers that too ([`Rehearsal::Unavailable`]), and the confirmed dispatch's [`Mutation`]
//! tells the handler not to send its own dry run first ([`Mutation::rehearses_first`]): it would
//! fail the same way, after the user confirmed.
//!
//! A person's own dry-run dispatch of a `resource::Apply` through the guard (the editor's apply
//! preview, `ApplyService::preview`) stands in for the rehearsal of the apply that follows it (see
//! `receipt`): that person has seen the server's answer, and nobody would see a second one.

use std::sync::Arc;

use oxikube_domain::audit::AuditOutcome;
use oxikube_domain::command::{Command, CommandMeta};
use oxikube_domain::safety::Risk;

use super::gate::ReadOnlyGate;
use super::pipeline::Admitted;
use super::preview::{
    DryRunResult, WriteLog, collect, drain_warnings, no_preview_reason, preview_verb,
};
use super::{Mutation, MutationGuard};
use crate::command_bus::{CommandHandler, DispatchContext, DispatchError, HandlerContext};

/// What the guard's dry run before a confirmation established: kept with the token and handed to
/// the handler of the confirmed dispatch through its [`Mutation`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Rehearsal {
    /// The guard dry-ran nothing (the command needs no dry run, or the dispatch is one itself).
    #[default]
    None,
    /// The server dry-ran exactly this request before it was confirmed.
    Done,
    /// The server cannot dry-run it ([`DryRunPreview::NoPreviewAvailable`]): the confirmed run
    /// sends the request without a dry run first.
    Unavailable,
}

impl Rehearsal {
    /// Whether the guard's dry run happened (or found that none is possible).
    pub(crate) fn happened(self) -> bool {
        self != Rehearsal::None
    }
}

impl From<&DryRunPreview> for Rehearsal {
    fn from(preview: &DryRunPreview) -> Self {
        match preview {
            DryRunPreview::Rehearsed(_) => Rehearsal::Done,
            DryRunPreview::NoPreviewAvailable { .. } => Rehearsal::Unavailable,
        }
    }
}

/// What the server-side dry run of a command showed, attached to its
/// [`ConfirmationRequest`](super::ConfirmationRequest).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DryRunPreview {
    /// The server accepted a dry run of exactly this command: what its handler reported (for a
    /// delete, `Dry run: namespace demo can be deleted`), each object it would write next to the
    /// live one, and the server's warnings.
    Rehearsed(DryRunResult),
    /// The server (an aggregated API, an older cluster, an admission webhook with side effects)
    /// does not support a dry run of this request. The mutation may still run once confirmed: a
    /// missing preview never blocks it.
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
/// the guard previews it ([`preview_verb`]) or its risk [requires it](Risk::requires_dry_run_diff),
/// and the dispatch is not a dry run itself.
pub(super) fn needs_rehearsal(command: &Command, ctx: &DispatchContext) -> bool {
    !ctx.dry_run
        && (preview_verb(command).is_some()
            || command
                .effective_risk()
                .is_some_and(Risk::requires_dry_run_diff))
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
    rehearsal: Rehearsal,
) -> Result<(), DispatchError> {
    if needs_rehearsal(command, ctx) && !rehearsal.happened() {
        return Err(DispatchError::DryRunRequired(meta.id));
    }
    Ok(())
}

impl MutationGuard {
    /// Runs `handler` for `command` once as a server-side dry run and returns what it showed: the
    /// objects it would write next to the live ones (Secret values masked) and the server's
    /// warnings.
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
        let log = Arc::new(WriteLog::default());
        let writer = Arc::new(ReadOnlyGate::rehearsal(
            self.sessions.clone(),
            admitted.cluster.clone(),
            admitted.context.clone(),
            admitted.writer.clone(),
            log.clone(),
        ));
        let session = self.sessions.get(&admitted.cluster);
        // Subscribed before the first request, so every warning the dry run draws is seen.
        let warnings = session
            .as_ref()
            .and_then(|session| session.warnings())
            .map(|port| port.subscribe());
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
        let warnings = drain_warnings(warnings);
        match result {
            Ok(output) => {
                let reader = session.and_then(|session| session.resources());
                Ok(DryRunPreview::Rehearsed(
                    collect(output.message, &log, reader, warnings).await,
                ))
            }
            Err(error) => match no_preview_reason(&error) {
                Some(reason) => {
                    tracing::info!(command = %meta.id, "no dry run available; asking without a preview");
                    Ok(DryRunPreview::NoPreviewAvailable { reason })
                }
                None => Err(DispatchError::Handler(error)),
            },
        }
    }
}
