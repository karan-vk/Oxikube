//! [`MutationGuard::run_posture`]: confirmation, audit and the handler for a posture command.

use std::sync::Arc;

use oxikube_domain::ClusterPreset;
use oxikube_domain::audit::AuditOutcome;
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::{Command, CommandMeta};
use oxikube_domain::safety::ConfirmTier;

use super::{Posture, lowers_protection, unflags_production};
use crate::command_bus::{CommandHandler, DispatchContext, DispatchError, HandlerContext, Outcome};
use crate::guard::confirm::{ConfirmationRequest, Pending};
use crate::guard::{MutationGuard, policy};

impl MutationGuard {
    /// Runs a posture command (see the [module docs](super)).
    pub(crate) async fn run_posture(
        &self,
        meta: &CommandMeta,
        command: Command,
        ctx: DispatchContext,
        handler: Arc<dyn CommandHandler>,
    ) -> Result<Outcome, DispatchError> {
        let Some(cluster) = policy::cluster_of(&command).cloned() else {
            return Err(DispatchError::NoCluster(meta.id));
        };
        let posture = Posture::of(&self.sessions, &cluster);
        let target = policy::audit_target(&command, &cluster);
        let lowers = lowers_protection(&command, &posture);

        // The production colour is what asks for the confirm above, so clearing it is a
        // person's explicit choice; an agent or a plugin may not.
        let unflags = unflags_production(&command, &posture);
        if unflags && matches!(ctx.initiator, Initiator::Agent | Initiator::Plugin) {
            let record = self.audit.entry(
                &ctx.who,
                ctx.initiator,
                meta.id.as_str(),
                target,
                ctx.dry_run,
                AuditOutcome::Denied,
            );
            if let Err(audit) = self.audit.record(record).await {
                tracing::warn!(error = %audit, "could not audit a refused posture change");
            }
            return Err(DispatchError::NotPermitted {
                command: meta.id,
                initiator: ctx.initiator,
            });
        }
        let weakens = lowers || unflags;

        // Lifting read-only on a cluster the user flagged as production asks first.
        if lowers && ClusterPreset::detect(posture.colour) == ClusterPreset::Prod {
            let label = posture.label(&cluster);
            match &ctx.confirmation {
                None => {
                    let token = self.confirmations.issue(Pending {
                        command: command.clone(),
                        initiator: ctx.initiator,
                        who: ctx.who.clone(),
                        cluster: cluster.clone(),
                        expected_name: None,
                        dry_run: ctx.dry_run,
                    });
                    return Ok(Outcome::NeedsConfirmation(ConfirmationRequest {
                        token,
                        command: meta.id,
                        tier: ConfirmTier::Simple,
                        risk: None,
                        cluster,
                        summary: format!(
                            "Turn off read-only mode on {label}: it is flagged as production"
                        ),
                        expected_name: None,
                    }));
                }
                Some(answer) => {
                    if let Err(err) =
                        self.confirmations
                            .redeem(answer, &command, ctx.initiator, ctx.dry_run)
                    {
                        let record = self.audit.entry(
                            &ctx.who,
                            ctx.initiator,
                            meta.id.as_str(),
                            target,
                            ctx.dry_run,
                            AuditOutcome::Denied,
                        );
                        if let Err(audit) = self.audit.record(record).await {
                            tracing::warn!(error = %audit, "could not audit a refused posture change");
                        }
                        return Err(DispatchError::Confirmation(err));
                    }
                }
            }
        }

        // Taking protection away needs a log to write to; adding it never waits on one.
        if weakens {
            self.audit
                .ensure_writable()
                .await
                .map_err(DispatchError::AuditUnavailable)?;
        }
        let attempt = self.audit.begin(
            &ctx.who,
            ctx.initiator,
            meta.id.as_str(),
            target,
            ctx.dry_run,
        );
        let cx = HandlerContext::new(ctx.initiator, ctx.who.clone(), Some(cluster), None)
            .with_dry_run(ctx.dry_run);
        let result = handler.handle(command, cx).await;
        attempt.finish(match result {
            Ok(_) => AuditOutcome::Succeeded,
            Err(_) => AuditOutcome::Failed,
        });
        match self.audit.flush().await {
            Ok(()) => {}
            Err(err) if weakens => return Err(DispatchError::AuditFailed(err)),
            Err(err) => tracing::warn!(error = %err, "posture change not audited yet; retrying"),
        }
        result
            .map(Outcome::Completed)
            .map_err(DispatchError::Handler)
    }
}
