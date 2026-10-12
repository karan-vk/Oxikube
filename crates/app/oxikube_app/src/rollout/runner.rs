//! [`RollbackRunner`]: runs `workload::Rollback` through the bus for a dialog that already asked.

use std::sync::Arc;

use oxikube_domain::audit::Initiator;
use oxikube_domain::command::Command;
use oxikube_domain::ids::ResourceRef;
use oxikube_domain::safety::ConfirmTier;
use oxikube_domain::{OxiError, OxiResult};

use super::result::RollbackOutcome;
use crate::command_bus::{CommandBus, DispatchContext, Outcome};
use crate::guard::Confirmation;

/// What a finished `workload::Rollback` says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RollbackReport {
    /// The line to show the user.
    pub message: String,
    /// The revision restored.
    pub to_revision: Option<i64>,
    /// Whether the workload already ran that template, so nothing was changed.
    pub already_current: bool,
}

/// Dispatches `workload::Rollback` on the bus as the user, answering the guard's confirmation on
/// the dialog's behalf. Cheap to clone. The view calls [`run`](Self::run) on the Tokio bridge
/// (`oxikube_runtime::spawn_kube`).
///
/// The rollback dialog is the user's confirmation (it names the workload, the revision, what
/// differs and the confirmation tier), so this answers the guard's simple confirmation once. It is
/// not a way around it: the command still passes the read-only check, the audit record and every
/// other stage, as `Initiator::Ui`.
#[derive(Clone)]
pub struct RollbackRunner {
    bus: CommandBus,
    who: Arc<str>,
}

impl std::fmt::Debug for RollbackRunner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RollbackRunner").finish_non_exhaustive()
    }
}

impl RollbackRunner {
    /// A runner dispatching on `bus` as `who` (the local user's name, for the audit log).
    pub fn new(bus: CommandBus, who: impl Into<Arc<str>>) -> Self {
        Self {
            bus,
            who: who.into(),
        }
    }

    /// Rolls `target` back to `to_revision` (`None`: the previous one).
    ///
    /// # Errors
    ///
    /// `Forbidden` for a read-only cluster or an account without the `patch` right, `Validation`
    /// for a kind without history or a revision that is gone, `Conflict` for a paused Deployment,
    /// and the handler's own error otherwise (the API server's message for an admission refusal).
    pub async fn run(
        &self,
        target: &ResourceRef,
        to_revision: Option<i64>,
    ) -> OxiResult<RollbackReport> {
        let command = Command::WorkloadRollback {
            target: target.clone(),
            to_revision,
        };
        let context = || {
            DispatchContext::new(Initiator::Ui, self.who.clone())
                .with_cluster(target.cluster.clone())
        };
        let outcome = self.bus.dispatch(command.clone(), context()).await?;
        let outcome = match outcome {
            Outcome::NeedsConfirmation(asked) => {
                // The dialog showed the simple confirmation; anything stricter is a person's.
                if asked.tier > ConfirmTier::Simple {
                    self.bus.decline(asked.token).await?;
                    return Err(OxiError::conflict(
                        "the confirmation this needs changed; try again",
                    ));
                }
                self.bus
                    .dispatch(
                        command,
                        context().with_confirmation(Confirmation::simple(asked.token)),
                    )
                    .await?
            }
            done => done,
        };
        let Outcome::Completed(output) = outcome else {
            return Err(OxiError::conflict("the confirmation was not accepted"));
        };
        let data = output.data.as_ref();
        Ok(RollbackReport {
            to_revision: data
                .and_then(|data| data.get("to_revision"))
                .and_then(|revision| revision.as_i64()),
            already_current: data
                .and_then(|data| data.get("outcome"))
                .and_then(|outcome| outcome.as_str())
                == Some(RollbackOutcome::AlreadyCurrent.as_str()),
            message: output
                .message
                .unwrap_or_else(|| "the workload was rolled back".to_owned()),
        })
    }
}
