//! The exec class: how a shell, an attach or an exec into a pod passes the guard.
//!
//! These commands ([`CommandMeta::exec`]) are not mutations, so they take no confirmation and no
//! [`Mutation`](super::Mutation), but they are as powerful as a shell is, so they do not run as
//! plain reads either:
//!
//! 1. **Read-only**: a read-only cluster refuses them for every initiator unless its
//!    `exec_in_read_only` setting ([`ClusterPrefs::exec_in_read_only`]) is on. A cluster with no
//!    open session is refused (fail closed).
//! 2. **Audit**: the open is audited like a mutation (`Succeeded`, `Failed`, `Denied`,
//!    `Cancelled`) with the initiator, the pod, and as the record's detail the session kind,
//!    container and program; never the typed input or the output. As for a mutation, a log that
//!    cannot be written refuses the open.
//! 3. **No confirmation**: the tier is [`ConfirmTier::None`](oxikube_domain::safety::ConfirmTier).
//!
//! The handler only requests the open (it queues it for the UI); a connection that then fails is
//! shown in the terminal tab, and what happens inside the session is not recorded.

use std::sync::Arc;

use oxikube_domain::audit::AuditOutcome;
use oxikube_domain::command::{Command, CommandMeta};
use oxikube_ports::ClusterPrefs;

use super::{MutationGuard, policy};
use crate::command_bus::{CommandHandler, DispatchContext, DispatchError, HandlerContext, Outcome};

/// The audit detail of an exec-class `command`: `session=shell container=app`, plus
/// `program=psql` for `pod::Exec` (the program only: its arguments may hold secrets). A
/// container left to the default reads `(default)`.
pub(super) fn detail_of(command: &Command) -> String {
    let container =
        |container: &Option<String>| container.as_deref().unwrap_or("(default)").to_owned();
    match command {
        Command::PodShell { container: c, .. } => {
            format!("session=shell container={}", container(c))
        }
        Command::PodAttach { container: c, .. } => {
            format!("session=attach container={}", container(c))
        }
        Command::PodExec {
            container: c,
            command,
            ..
        } => {
            let program = command.first().map_or("(shell)", String::as_str);
            format!("session=exec container={} program={program}", container(c))
        }
        _ => "session=unknown".to_owned(),
    }
}

/// Whether `prefs` lets an exec-class command run on a read-only cluster.
fn allowed_read_only(prefs: &ClusterPrefs) -> bool {
    prefs.exec_in_read_only
}

impl MutationGuard {
    /// Runs an exec-class `command` described by `meta` through the exec policy (see the
    /// [module docs](self)).
    pub(crate) async fn run_exec(
        &self,
        meta: &CommandMeta,
        command: Command,
        ctx: DispatchContext,
        handler: Arc<dyn CommandHandler>,
    ) -> Result<Outcome, DispatchError> {
        let Some(cluster) = policy::cluster_of(&command)
            .or(ctx.cluster.as_ref())
            .cloned()
        else {
            return Err(DispatchError::NoCluster(meta.id));
        };
        let target = policy::audit_target(&command, &cluster);
        let detail = detail_of(&command);
        let refused = |error: DispatchError, outcome: AuditOutcome| {
            let record = self.audit.entry_with_detail(
                &ctx.who,
                ctx.initiator,
                meta.id.as_str(),
                target.clone(),
                &detail,
                outcome,
            );
            (error, record)
        };

        let denial = match self.sessions.get(&cluster) {
            None => Some(refused(
                DispatchError::NoSession(cluster.clone()),
                AuditOutcome::Denied,
            )),
            Some(session) if session.read_only() && !allowed_read_only(session.prefs()) => {
                Some(refused(
                    DispatchError::ReadOnly {
                        cluster: cluster.clone(),
                        context: session.context().clone(),
                    },
                    AuditOutcome::Denied,
                ))
            }
            Some(session) if session.exec().is_none() => Some(refused(
                DispatchError::NotConnected {
                    cluster: cluster.clone(),
                    context: session.context().clone(),
                },
                AuditOutcome::Failed,
            )),
            Some(_) => None,
        };
        if let Some((error, record)) = denial {
            if let Err(err) = self.audit.record(record).await {
                tracing::warn!(error = %err, "could not audit a refused exec");
            }
            return Err(error);
        }

        self.audit
            .ensure_writable()
            .await
            .map_err(DispatchError::AuditUnavailable)?;
        // Armed before the handler, as for a mutation: a dropped dispatch is still audited.
        let attempt = self.audit.begin_with_detail(
            &ctx.who,
            ctx.initiator,
            meta.id.as_str(),
            target,
            &detail,
        );
        let cx = HandlerContext::new(ctx.initiator, ctx.who.clone(), Some(cluster), None);
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
}
