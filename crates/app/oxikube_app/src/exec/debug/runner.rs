//! [`DebugRunner`]: runs `pod::Debug` through the bus for a dialog that already asked the user.

use std::sync::Arc;

use oxikube_domain::audit::Initiator;
use oxikube_domain::safety::ConfirmTier;
use oxikube_domain::{OxiError, OxiResult};

use super::request::DebugRequest;
use crate::command_bus::{CommandBus, DispatchContext, Outcome};
use crate::guard::Confirmation;

/// What a finished `pod::Debug` says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebugReport {
    /// The new container's name (empty for a dry run, which adds none).
    pub container: String,
    /// The line to show the user.
    pub message: String,
}

/// Dispatches `pod::Debug` on the bus as the user, answering the guard's confirmation on the
/// dialog's behalf. Cheap to clone. The view calls [`run`](Self::run) on the Tokio bridge
/// (`oxikube_runtime::spawn_kube`): the command waits for the container to run.
///
/// The dialog is the user's confirmation (it names the pod, the image and the target, and says the
/// container cannot be removed), so this answers the guard's simple confirmation once. It is not
/// a way around it: the command still passes the read-only check, the audit record and every
/// other stage, as `Initiator::Ui`.
#[derive(Clone)]
pub struct DebugRunner {
    bus: CommandBus,
    who: Arc<str>,
}

impl std::fmt::Debug for DebugRunner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DebugRunner").finish_non_exhaustive()
    }
}

impl DebugRunner {
    /// A runner dispatching on `bus` as `who` (the local user's name, for the audit log).
    pub fn new(bus: CommandBus, who: impl Into<Arc<str>>) -> Self {
        Self {
            bus,
            who: who.into(),
        }
    }

    /// Adds the debug container and opens its terminal (the `pod::Debug` handler queues the tab).
    ///
    /// # Errors
    ///
    /// `Validation` for a request with a bad field (nothing is sent), `Forbidden` for a read-only
    /// cluster or an account that may not patch the pod, and the handler's own error otherwise:
    /// the API server's message when it refuses the container, `Timeout` when it did not run.
    pub async fn run(&self, request: &DebugRequest) -> OxiResult<DebugReport> {
        request.check_fields()?;
        let command = request.to_command();
        let context = || {
            DispatchContext::new(Initiator::Ui, self.who.clone())
                .with_cluster(request.pod.cluster.clone())
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
        let container = output
            .data
            .as_ref()
            .and_then(|data| data.get("container"))
            .and_then(|name| name.as_str())
            .unwrap_or_default()
            .to_owned();
        Ok(DebugReport {
            message: output
                .message
                .unwrap_or_else(|| "the debug container is running".to_owned()),
            container,
        })
    }
}
