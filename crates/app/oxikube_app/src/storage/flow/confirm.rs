//! The dispatch both storage flows share: send the command, answer the guard's simple
//! confirmation for the dialog's confirm button, and refuse a request that asks for more.

use std::sync::Arc;

use oxikube_domain::OxiError;
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::Command;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::safety::ConfirmTier;

use crate::CommandBus;
use crate::command_bus::{CommandOutput, DispatchContext, DispatchError, Outcome};
use crate::guard::Confirmation;

/// Dispatches `command` on `cluster` as the user (`who`, [`Initiator::Ui`]) and confirms the
/// guard's request, which must be a [`ConfirmTier::Simple`] one: the dialog the user clicked in
/// promised a one-click confirmation, so a request for a typed name (the policy moved under us)
/// is declined instead of answered.
pub(super) async fn dispatch_confirmed(
    bus: &CommandBus,
    who: &Arc<str>,
    cluster: &ClusterId,
    command: Command,
) -> Result<CommandOutput, DispatchError> {
    let context = || DispatchContext::new(Initiator::Ui, who.clone()).with_cluster(cluster.clone());
    let outcome = bus.dispatch(command.clone(), context()).await?;
    let output = match outcome {
        Outcome::Completed(output) => return Ok(output),
        Outcome::NeedsConfirmation(request) => {
            if request.tier > ConfirmTier::Simple {
                bus.decline(request.token).await?;
                return Err(DispatchError::Handler(OxiError::conflict(
                    "the confirmation this action needs changed; try again",
                )));
            }
            bus.dispatch(
                command,
                context().with_confirmation(Confirmation::simple(request.token)),
            )
            .await?
        }
    };
    match output {
        Outcome::Completed(output) => Ok(output),
        Outcome::NeedsConfirmation(_) => Err(DispatchError::Handler(OxiError::conflict(
            "the confirmation was not accepted",
        ))),
    }
}
