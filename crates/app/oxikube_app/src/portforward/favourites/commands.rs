//! `portforward::FavouriteAdd`, `FavouriteRemove` and `FavouriteToggleAutoStart` on the command
//! bus (each with its MCP tool stub). They change the app's own saved state only: no cluster is
//! touched, so no guard tier applies.

use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::portforward::ForwardSpec;
use oxikube_domain::{OxiError, OxiResult};
use serde_json::json;

use super::model::FavouriteId;
use super::service::PortForwardFavourites;
use crate::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};

/// The commands [`register_commands`] installs.
pub const COMMANDS: [CommandId; 3] = [
    CommandId::PORTFORWARD_FAVOURITE_ADD,
    CommandId::PORTFORWARD_FAVOURITE_REMOVE,
    CommandId::PORTFORWARD_FAVOURITE_TOGGLE_AUTO_START,
];

/// Registers the favourites' commands on `registry`.
///
/// # Errors
///
/// A [`RegisterError`] when one of the ids is already taken.
pub fn register_commands(
    registry: &mut CommandRegistry,
    favourites: PortForwardFavourites,
) -> Result<(), RegisterError> {
    for id in COMMANDS {
        let favourites = favourites.clone();
        let meta = *command::lookup(id).expect("declared in oxikube_domain::command");
        registry.register(meta, move |command: Command, _: HandlerContext| {
            let favourites = favourites.clone();
            async move { run(&favourites, command).await }
        })?;
    }
    Ok(())
}

pub(super) async fn run(
    favourites: &PortForwardFavourites,
    command: Command,
) -> OxiResult<CommandOutput> {
    match command {
        Command::PortForwardFavouriteAdd {
            target,
            remote_port,
            local_port,
            bind,
            name,
            auto_start,
        } => {
            let mut spec = ForwardSpec::new(target, remote_port);
            spec.local_port = local_port.unwrap_or(0);
            if let Some(bind) = bind {
                spec.bind = bind;
            }
            let favourite = favourites
                .add(spec, name, auto_start.unwrap_or(true))
                .await?;
            Ok(CommandOutput {
                message: Some(format!("Saved favourite {}", favourite.name)),
                data: Some(json!({
                    "id": favourite.id.as_str(),
                    "name": favourite.name,
                    "auto_start": favourite.auto_start,
                })),
            })
        }
        Command::PortForwardFavouriteRemove { id } => {
            let id = FavouriteId::new(id);
            if !favourites.remove(&id).await? {
                return Err(unknown(&id));
            }
            Ok(CommandOutput {
                message: Some(format!("Removed favourite {id}")),
                data: Some(json!({ "id": id.as_str() })),
            })
        }
        Command::PortForwardFavouriteToggleAutoStart { id } => {
            let id = FavouriteId::new(id);
            let auto_start = favourites.toggle_auto_start(&id).await?;
            Ok(CommandOutput {
                message: Some(format!(
                    "Favourite {id} {} on connect",
                    if auto_start {
                        "starts"
                    } else {
                        "does not start"
                    }
                )),
                data: Some(json!({ "id": id.as_str(), "auto_start": auto_start })),
            })
        }
        other => Err(OxiError::internal(format!(
            "{} is not a port-forward favourite command",
            other.id()
        ))),
    }
}

fn unknown(id: &FavouriteId) -> OxiError {
    OxiError::not_found(format!("no port-forward favourite {id}"))
}
