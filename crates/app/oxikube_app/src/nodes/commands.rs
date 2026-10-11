//! The `node::Cordon` and `node::Uncordon` handlers.

use oxikube_domain::OxiError;
use oxikube_domain::command::{self, Command, CommandId};
use serde_json::json;

use super::service::{NodeOutcome, set_unschedulable};
use crate::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};

/// Registers `node::Cordon` and `node::Uncordon` on `registry` (the binary installs them under
/// `oxikube_app::nodes`).
///
/// Both run only after the guard let the command through: read-only check, confirmation and audit
/// precondition are the guard's. Each is a [`set_unschedulable`] call on the permit the guard
/// gives the handler.
///
/// # Errors
///
/// A [`RegisterError`] when either command is registered already.
pub fn register_commands(registry: &mut CommandRegistry) -> Result<(), RegisterError> {
    for id in [CommandId::NODE_CORDON, CommandId::NODE_UNCORDON] {
        let meta = *command::lookup(id).ok_or(RegisterError::Undeclared(id))?;
        registry.register(meta, |command: Command, cx: HandlerContext| async move {
            let (target, cordon) = match command {
                Command::NodeCordon { target } => (target, true),
                Command::NodeUncordon { target } => (target, false),
                _ => return Err(OxiError::validation("not a node::Cordon or node::Uncordon")),
            };
            let mutation = cx.require_mutation()?;
            let outcome = set_unschedulable(mutation, &target, cordon).await?;
            let (done, verb) = if cordon {
                ("Cordoned", "cordoned")
            } else {
                ("Uncordoned", "uncordoned")
            };
            Ok(match outcome {
                NodeOutcome::Applied => CommandOutput {
                    message: Some(format!("{done} node {}", target.name)),
                    data: Some(json!({ "outcome": verb })),
                },
                NodeOutcome::DryRun => CommandOutput {
                    message: Some(format!("Dry run: node {} can be {verb}", target.name)),
                    data: Some(json!({ "outcome": "dry_run" })),
                },
            })
        })?;
    }
    Ok(())
}
