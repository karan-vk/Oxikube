//! [`ScaleRunner`]: runs `workload::Scale` through the bus for a dialog that already asked.

use std::sync::Arc;

use oxikube_domain::audit::Initiator;
use oxikube_domain::command::Command;
use oxikube_domain::ids::ResourceRef;
use oxikube_domain::safety::ConfirmTier;
use oxikube_domain::{OxiError, OxiResult};

use crate::command_bus::{CommandBus, DispatchContext, Outcome};
use crate::guard::Confirmation;

/// What a finished `workload::Scale` says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScaleReport {
    /// The line to show the user.
    pub message: String,
    /// `spec.replicas` after.
    pub after: Option<u64>,
}

/// Dispatches `workload::Scale` on the bus as the user, answering the guard's confirmation on the
/// dialog's behalf. Cheap to clone. The view calls [`run`](Self::run) on the Tokio bridge
/// (`oxikube_runtime::spawn_kube`).
///
/// The scale dialog is the user's confirmation (it names the workload, the current and the new
/// count, and warns about zero), so this answers the guard's simple confirmation once. It is not
/// a way around it: the command still passes the read-only check, the audit record and every other
/// stage, as `Initiator::Ui`.
#[derive(Clone)]
pub struct ScaleRunner {
    bus: CommandBus,
    who: Arc<str>,
}

impl std::fmt::Debug for ScaleRunner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScaleRunner").finish_non_exhaustive()
    }
}

impl ScaleRunner {
    /// A runner dispatching on `bus` as `who` (the local user's name, for the audit log).
    pub fn new(bus: CommandBus, who: impl Into<Arc<str>>) -> Self {
        Self {
            bus,
            who: who.into(),
        }
    }

    /// Scales `target` to `replicas`.
    ///
    /// # Errors
    ///
    /// `Forbidden` for a read-only cluster or an account without the `scale` right, `Validation`
    /// for a kind that cannot be scaled, and the handler's own error otherwise (the API server's
    /// message for an admission refusal).
    pub async fn run(&self, target: &ResourceRef, replicas: u32) -> OxiResult<ScaleReport> {
        let command = Command::WorkloadScale {
            target: target.clone(),
            replicas,
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
        Ok(ScaleReport {
            after: output
                .data
                .as_ref()
                .and_then(|data| data.get("after"))
                .and_then(|after| after.as_u64()),
            message: output
                .message
                .unwrap_or_else(|| "the workload was scaled".to_owned()),
        })
    }
}
