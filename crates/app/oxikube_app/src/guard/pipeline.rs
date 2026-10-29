//! The guard pipeline itself: admission (synchronous), the dry run an `Irreversible` command
//! needs before it is confirmed (see `rehearsal`), then execute and audit.

use std::sync::Arc;

use oxikube_domain::audit::{AuditOutcome, AuditRecord};
use oxikube_domain::command::{Command, CommandMeta};
use oxikube_domain::ids::{ClusterId, ContextName, ResourceRef};
use oxikube_domain::safety::ConfirmTier;
use oxikube_ports::ResourceWriter;

use super::confirm::{ConfirmationRequest, Pending};
use super::gate::ReadOnlyGate;
use super::rehearsal::{needs_rehearsal, require_rehearsal};
use super::{Mutation, MutationGuard, policy};
use crate::audit::AuditLog;
use crate::command_bus::{CommandHandler, DispatchContext, DispatchError, HandlerContext, Outcome};

/// What the synchronous checks let through: where the command will write and what its audit
/// record names.
pub(super) struct Admitted {
    pub(super) cluster: ClusterId,
    pub(super) context: ContextName,
    pub(super) target: ResourceRef,
    pub(super) writer: Arc<dyn ResourceWriter>,
    /// What the audit record adds to the target ([`policy::audit_detail`]).
    pub(super) detail: Option<String>,
}

/// A confirmation to ask for, before its token is issued.
struct Question {
    admitted: Admitted,
    pending: Pending,
    tier: ConfirmTier,
    summary: String,
}

/// The result of the synchronous checks.
enum Admission {
    /// Run the handler. `rehearsed`: the confirmation redeemed followed the guard's dry run.
    Proceed { admitted: Admitted, rehearsed: bool },
    /// Ask the user first; nothing ran. The token is issued once any dry run is done.
    Confirm(Question),
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
        let resolved = self.resolved_target(&command, ctx.cluster.as_ref()).await;
        let resolved_detail = self.resolved_detail(&command, ctx.cluster.as_ref()).await;
        let (admitted, rehearsed) =
            match self.admit(meta, &command, &ctx, resolved, resolved_detail) {
                Admission::Proceed {
                    admitted,
                    rehearsed,
                } => (admitted, rehearsed),
                Admission::Confirm(question) => {
                    return self.ask(meta, &command, &ctx, question, handler).await;
                }
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

        // The dry-run stage: a command that needs the guard's dry run got it before it was
        // confirmed (`ask`), or it does not run.
        if let Err(error) = require_rehearsal(meta, &command, &ctx, rehearsed) {
            let record = self.refusal(meta, &ctx, &admitted, &error);
            if let Err(err) = self.audit.record(record).await {
                tracing::warn!(error = %err, "could not audit a refused mutation");
            }
            return Err(error);
        }

        self.audit
            .ensure_writable()
            .await
            .map_err(DispatchError::AuditUnavailable)?;

        let Admitted {
            cluster,
            context,
            target,
            writer,
            detail,
        } = admitted;
        // Armed before the handler: if this future is dropped while the handler runs,
        // the attempt still lands in the audit backlog (as `Cancelled`).
        let attempt = self.audit.begin_described(
            &ctx.who,
            ctx.initiator,
            meta.id.as_str(),
            target,
            ctx.dry_run,
            detail.as_deref(),
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
        )
        .with_audit(self.audit.clone());
        let note = cx.audit_note_slot();
        // A handler dropped mid-way (a cancelled drain) still leaves what it noted on the
        // `Cancelled` record.
        let attempt = attempt.with_note_slot(note.clone());
        let result = handler.handle(command, cx).await;

        let note = note.lock().ok().and_then(|mut slot| slot.take());
        attempt.finish_noting(
            match result {
                Ok(_) => AuditOutcome::Succeeded,
                Err(_) => AuditOutcome::Failed,
            },
            note.as_deref(),
        );
        self.audit
            .flush()
            .await
            .map_err(DispatchError::AuditFailed)?;
        result
            .map(Outcome::Completed)
            .map_err(DispatchError::Handler)
    }

    /// Answers the first dispatch of a command that needs confirming: dry-runs it first when its
    /// risk requires that (see `rehearsal`), then issues the token and returns the request.
    async fn ask(
        &self,
        meta: &CommandMeta,
        command: &Command,
        ctx: &DispatchContext,
        question: Question,
        handler: Arc<dyn CommandHandler>,
    ) -> Result<Outcome, DispatchError> {
        let Question {
            admitted,
            pending,
            tier,
            summary,
        } = question;
        let preview = if needs_rehearsal(command, ctx) {
            Some(
                self.rehearse(meta, command, ctx, &admitted, handler)
                    .await?,
            )
        } else {
            None
        };
        let expected_name = pending.expected_name.clone();
        let token = self.confirmations.issue(Pending {
            rehearsed: preview.is_some(),
            ..pending
        });
        Ok(Outcome::NeedsConfirmation(ConfirmationRequest {
            token,
            command: meta.id,
            tier,
            risk: command.effective_risk(),
            cluster: admitted.cluster,
            summary,
            expected_name,
            initiator: ctx.initiator,
            preview,
        }))
    }

    /// The `Denied` record of `error` refusing an admitted command.
    fn refusal(
        &self,
        meta: &CommandMeta,
        ctx: &DispatchContext,
        admitted: &Admitted,
        error: &DispatchError,
    ) -> AuditRecord {
        let record = AuditLog::describe(
            self.audit.entry(
                &ctx.who,
                ctx.initiator,
                meta.id.as_str(),
                admitted.target.clone(),
                ctx.dry_run,
                AuditOutcome::Denied,
            ),
            admitted.detail.as_deref(),
        );
        match error.audit_denial() {
            Some(denial) => record.with_denial(denial),
            None => record,
        }
    }

    /// The audit target of a `resource::Apply`, placed by the cluster's discovery as the handler
    /// places the object (an object given no namespace is in `default`); `None` for any other
    /// command, which [`policy::audit_target`] names from its payload.
    pub(super) async fn resolved_target(
        &self,
        command: &Command,
        context_cluster: Option<&ClusterId>,
    ) -> Option<ResourceRef> {
        let cluster = policy::cluster_of(command).or(context_cluster)?;
        crate::apply::resolved_audit_target(command, cluster, &self.sessions).await
    }

    /// What a `pvc::Resize` that does not name its old size should say in its audit record: the
    /// live size, read as the handler reads it; `None` for any other command.
    pub(super) async fn resolved_detail(
        &self,
        command: &Command,
        context_cluster: Option<&ClusterId>,
    ) -> Option<String> {
        let cluster = policy::cluster_of(command).or(context_cluster)?;
        crate::storage::resolved_audit_detail(command, cluster, &self.sessions).await
    }

    /// The synchronous, in-memory checks: cluster, session, read-only, connection,
    /// confirmation. `resolved` is the audit target and `resolved_detail` the audit detail when
    /// they were looked up already ([`resolved_target`](Self::resolved_target),
    /// [`resolved_detail`](Self::resolved_detail)).
    fn admit(
        &self,
        meta: &CommandMeta,
        command: &Command,
        ctx: &DispatchContext,
        resolved: Option<ResourceRef>,
        resolved_detail: Option<String>,
    ) -> Admission {
        let Some(cluster) = policy::cluster_of(command)
            .or(ctx.cluster.as_ref())
            .cloned()
        else {
            return Admission::Deny {
                error: DispatchError::NoCluster(meta.id),
                record: None,
            };
        };
        let target = resolved.unwrap_or_else(|| policy::audit_target(command, &cluster));
        let detail = resolved_detail
            .clone()
            .or_else(|| policy::audit_detail(command));
        let deny = |error: DispatchError, outcome: AuditOutcome| {
            let record = AuditLog::describe(
                self.audit.entry(
                    &ctx.who,
                    ctx.initiator,
                    meta.id.as_str(),
                    target.clone(),
                    ctx.dry_run,
                    outcome,
                ),
                detail.as_deref(),
            );
            let record = match error.audit_denial() {
                Some(denial) if outcome == AuditOutcome::Denied => record.with_denial(denial),
                _ => record,
            };
            Admission::Deny {
                error,
                record: Some(record),
            }
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

        let tier = policy::confirm_tier_for(meta, command);
        let mut rehearsed = false;
        if tier != ConfirmTier::None
            && let Some(answer) = &ctx.confirmation
        {
            match self
                .confirmations
                .redeem(answer, command, ctx.initiator, ctx.dry_run)
            {
                Ok(dry_ran) => rehearsed = dry_ran,
                Err(err) => {
                    return deny(DispatchError::Confirmation(err), AuditOutcome::Denied);
                }
            }
        }

        let summary = (tier != ConfirmTier::None && ctx.confirmation.is_none())
            .then(|| policy::summary_in(meta, command, context.as_str(), session.prefs()));
        let admitted = Admitted {
            detail: resolved_detail.or_else(|| policy::audit_detail_in(command, session.prefs())),
            cluster,
            context,
            target,
            writer,
        };
        match summary {
            Some(summary) => {
                let expected_name = (tier == ConfirmTier::TypeName)
                    .then(|| policy::expected_name(command, admitted.context.as_str()));
                Admission::Confirm(Question {
                    pending: Pending {
                        command: command.clone(),
                        initiator: ctx.initiator,
                        who: ctx.who.clone(),
                        cluster: admitted.cluster.clone(),
                        expected_name,
                        dry_run: ctx.dry_run,
                        rehearsed: false,
                    },
                    admitted,
                    tier,
                    summary,
                })
            }
            None => Admission::Proceed {
                admitted,
                rehearsed,
            },
        }
    }
}
