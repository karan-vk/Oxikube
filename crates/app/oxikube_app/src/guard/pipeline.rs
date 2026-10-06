//! The guard pipeline itself: admission (synchronous), then execute and audit.

use std::sync::Arc;

use oxikube_domain::audit::{AuditOutcome, AuditRecord};
use oxikube_domain::command::{Command, CommandMeta};
use oxikube_domain::ids::{ClusterId, ContextName, ResourceRef};
use oxikube_domain::safety::ConfirmTier;
use oxikube_ports::ResourceWriter;

use super::confirm::{ConfirmationError, ConfirmationRequest, ConfirmationToken, Pending};
use super::gate::ReadOnlyGate;
use super::{Mutation, MutationGuard, policy};
use crate::command_bus::{CommandHandler, DispatchContext, DispatchError, HandlerContext, Outcome};

/// The result of the synchronous checks.
enum Admission {
    /// Run the handler.
    Proceed {
        cluster: ClusterId,
        context: ContextName,
        target: ResourceRef,
        writer: Arc<dyn ResourceWriter>,
    },
    /// Ask the user first; nothing ran.
    Confirm(ConfirmationRequest),
    /// Refused; `record` is written before the error is returned.
    Deny {
        error: DispatchError,
        record: Option<AuditRecord>,
    },
}

impl MutationGuard {
    /// Runs a mutating `command` described by `meta` through the pipeline.
    pub(crate) async fn run(
        &self,
        meta: &CommandMeta,
        command: Command,
        ctx: DispatchContext,
        handler: Arc<dyn CommandHandler>,
    ) -> Result<Outcome, DispatchError> {
        let (cluster, context, target, writer) = match self.admit(meta, &command, &ctx) {
            Admission::Proceed {
                cluster,
                context,
                target,
                writer,
            } => (cluster, context, target, writer),
            Admission::Confirm(request) => return Ok(Outcome::NeedsConfirmation(request)),
            Admission::Deny { error, record } => {
                if let Some(record) = record
                    && let Err(err) = self.audit.record(record).await
                {
                    // The refusal stands; the record stays in the backlog and blocks
                    // the next mutation until it is written.
                    tracing::warn!(error = %err, "could not audit a refused mutation");
                }
                return Err(error);
            }
        };

        dry_run_stage(meta);

        self.audit
            .ensure_writable()
            .await
            .map_err(DispatchError::AuditUnavailable)?;

        // Armed before the handler: if this future is dropped while the handler runs,
        // the attempt still lands in the audit backlog (as `Cancelled`).
        let attempt = self.audit.begin(
            &ctx.who,
            ctx.initiator,
            meta.id.as_str(),
            target,
            ctx.dry_run,
        );
        // The second read-only check: the writer re-reads the flag before every request, so a
        // flow that outlives the admission check still stops when read-only mode goes on.
        let writer = Arc::new(ReadOnlyGate::new(
            self.sessions.clone(),
            cluster.clone(),
            context,
            writer,
        ));
        let mutation = Mutation::new(cluster.clone(), writer, ctx.dry_run);
        let cx = HandlerContext::new(
            ctx.initiator,
            ctx.who.clone(),
            Some(cluster),
            Some(mutation),
        );
        let result = handler.handle(command, cx).await;

        attempt.finish(match result {
            Ok(_) => AuditOutcome::Succeeded,
            Err(_) => AuditOutcome::Failed,
        });
        self.audit
            .flush()
            .await
            .map_err(DispatchError::AuditFailed)?;
        result
            .map(Outcome::Completed)
            .map_err(DispatchError::Handler)
    }

    /// Declines a pending confirmation and audits it as `Cancelled`.
    pub(crate) async fn decline(&self, token: ConfirmationToken) -> Result<(), DispatchError> {
        let pending = self
            .confirmations
            .take(token)
            .ok_or(ConfirmationError::Unknown)?;
        let target = policy::audit_target(&pending.command, &pending.cluster);
        let record = self.audit.entry(
            &pending.who,
            pending.initiator,
            pending.command.id().as_str(),
            target,
            pending.dry_run,
            AuditOutcome::Cancelled,
        );
        self.audit
            .record(record)
            .await
            .map_err(DispatchError::AuditFailed)
    }

    /// The synchronous, in-memory checks: cluster, session, read-only, connection,
    /// confirmation.
    fn admit(&self, meta: &CommandMeta, command: &Command, ctx: &DispatchContext) -> Admission {
        let Some(cluster) = policy::cluster_of(command)
            .or(ctx.cluster.as_ref())
            .cloned()
        else {
            return Admission::Deny {
                error: DispatchError::NoCluster(meta.id),
                record: None,
            };
        };
        let target = policy::audit_target(command, &cluster);
        let deny = |error: DispatchError, outcome: AuditOutcome| Admission::Deny {
            error,
            record: Some(self.audit.entry(
                &ctx.who,
                ctx.initiator,
                meta.id.as_str(),
                target.clone(),
                ctx.dry_run,
                outcome,
            )),
        };

        let Some(session) = self.sessions.get(&cluster) else {
            return deny(DispatchError::NoSession(cluster), AuditOutcome::Denied);
        };
        let context = session.context().clone();
        if session.read_only() {
            return deny(
                DispatchError::ReadOnly { cluster, context },
                AuditOutcome::Denied,
            );
        }
        let Some(writer) = session.writer() else {
            return deny(
                DispatchError::NotConnected { cluster, context },
                AuditOutcome::Failed,
            );
        };

        let tier = policy::confirm_tier(meta);
        if tier != ConfirmTier::None {
            match &ctx.confirmation {
                None => {
                    let expected_name = (tier == ConfirmTier::TypeName)
                        .then(|| policy::expected_name(command, context.as_str()));
                    let token = self.confirmations.issue(Pending {
                        command: command.clone(),
                        initiator: ctx.initiator,
                        who: ctx.who.clone(),
                        cluster: cluster.clone(),
                        expected_name: expected_name.clone(),
                        dry_run: ctx.dry_run,
                    });
                    return Admission::Confirm(ConfirmationRequest {
                        token,
                        command: meta.id,
                        tier,
                        risk: meta.risk,
                        cluster,
                        summary: policy::summary(meta, command, context.as_str()),
                        expected_name,
                    });
                }
                Some(answer) => {
                    if let Err(err) =
                        self.confirmations
                            .redeem(answer, command, ctx.initiator, ctx.dry_run)
                    {
                        return deny(DispatchError::Confirmation(err), AuditOutcome::Denied);
                    }
                }
            }
        }

        Admission::Proceed {
            cluster,
            context,
            target,
            writer,
        }
    }
}

/// The dry-run stage. E19 runs a server-side dry run and shows its diff before every
/// command whose risk [requires it](oxikube_domain::safety::Risk::requires_dry_run_diff);
/// this story only marks the place in the pipeline.
fn dry_run_stage(meta: &CommandMeta) {
    if meta.risk.is_some_and(|r| r.requires_dry_run_diff()) {
        tracing::debug!(command = %meta.id, "dry-run diff stage is a stub until E19");
    }
}
