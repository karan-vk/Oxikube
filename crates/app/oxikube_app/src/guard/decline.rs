//! Declining a confirmation: by the user ([`MutationGuard::decline`]) or because nobody is left
//! to answer it ([`MutationGuard::abandon`]). Either way nothing ran, and the attempt is audited
//! as `Cancelled` with [`AuditDenial::UserCancelled`].

use oxikube_domain::audit::{AuditDenial, AuditOutcome, AuditRecord};
use oxikube_domain::ids::ResourceRef;

use super::confirm::{ConfirmationError, ConfirmationToken, Pending};
use super::{MutationGuard, policy};
use crate::audit::AuditLog;
use crate::command_bus::DispatchError;

impl MutationGuard {
    /// Declines a pending confirmation and audits it as `Cancelled` by the user.
    pub(crate) async fn decline(&self, token: ConfirmationToken) -> Result<(), DispatchError> {
        let pending = self
            .confirmations
            .take(token)
            .ok_or(ConfirmationError::Unknown)?;
        let target = match self
            .resolved_target(&pending.command, Some(&pending.cluster))
            .await
        {
            Some(target) => target,
            None => policy::audit_target(&pending.command, &pending.cluster),
        };
        let detail = match self
            .resolved_detail(&pending.command, Some(&pending.cluster))
            .await
        {
            Some(detail) => Some(detail),
            None => policy::audit_detail(&pending.command),
        };
        self.audit
            .record(self.declined(&pending, target, detail.as_deref()))
            .await
            .map_err(DispatchError::AuditFailed)
    }

    /// Forgets a pending confirmation nobody will answer (the task waiting on a
    /// [`ConfirmationProvider`](super::ConfirmationProvider) was dropped) and queues its
    /// `Cancelled` record, synchronously: the next flush writes it. Does nothing for a token that
    /// is no longer pending.
    pub(crate) fn abandon(&self, token: ConfirmationToken) {
        let Some(pending) = self.confirmations.take(token) else {
            return;
        };
        let target = policy::audit_target(&pending.command, &pending.cluster);
        let detail = policy::audit_detail(&pending.command);
        self.audit
            .enqueue(self.declined(&pending, target, detail.as_deref()));
    }

    /// The record of `pending` declined by the user.
    fn declined(
        &self,
        pending: &Pending,
        target: ResourceRef,
        detail: Option<&str>,
    ) -> AuditRecord {
        AuditLog::describe(
            self.audit.entry(
                &pending.who,
                pending.initiator,
                pending.command.id().as_str(),
                target,
                pending.dry_run,
                AuditOutcome::Cancelled,
            ),
            detail,
        )
        .with_denial(AuditDenial::UserCancelled)
    }
}
