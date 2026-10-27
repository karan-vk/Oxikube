//! `notifications::MarkRead`, `notifications::MarkAllRead`, `notifications::MuteCluster` and
//! `notifications::UnmuteCluster` on the command bus, each with its MCP tool stub. The fifth
//! command, `notifications::TogglePanel`, opens a view, so the window registers it
//! (`oxikube_workspace::notifications`).
//!
//! None changes a cluster: they touch the app's own list and the local state db, so none goes
//! through a guard tier and all run on a read-only cluster.

use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::{OxiError, OxiResult};
use serde_json::json;

use super::model::NotificationId;
use super::service::NotificationService;
use crate::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};

/// The commands [`register_commands`] installs.
pub const COMMANDS: [CommandId; 4] = [
    CommandId::NOTIFICATIONS_MARK_ALL_READ,
    CommandId::NOTIFICATIONS_MARK_READ,
    CommandId::NOTIFICATIONS_MUTE_CLUSTER,
    CommandId::NOTIFICATIONS_UNMUTE_CLUSTER,
];

/// Registers the centre's commands on `registry`. The binary installs it as
/// `oxikube_app::notifications`.
///
/// # Errors
///
/// A [`RegisterError`] when one of the ids is already taken.
///
/// # Panics
///
/// When one of the ids is not declared in `oxikube_domain::command`, a bug in this module.
pub fn register_commands(
    registry: &mut CommandRegistry,
    service: NotificationService,
) -> Result<(), RegisterError> {
    for id in COMMANDS {
        let service = service.clone();
        let meta = *command::lookup(id).expect("declared in oxikube_domain::command");
        registry.register(meta, move |command: Command, _: HandlerContext| {
            let service = service.clone();
            async move { run(&service, command).await }
        })?;
    }
    Ok(())
}

async fn run(service: &NotificationService, command: Command) -> OxiResult<CommandOutput> {
    let output = match command {
        Command::NotificationsMarkRead { id } => {
            let marked = service.mark_read(NotificationId(id));
            if !marked && service.get(NotificationId(id)).is_none() {
                return Err(OxiError::not_found(format!("no notification {id}")));
            }
            CommandOutput {
                message: None,
                data: Some(json!({ "id": id, "marked": marked, "unread": service.unread() })),
            }
        }
        Command::NotificationsMarkAllRead => {
            let marked = service.mark_all_read();
            CommandOutput {
                message: None,
                data: Some(json!({ "marked": marked, "unread": 0 })),
            }
        }
        Command::NotificationsMuteCluster { cluster } => {
            let changed = service.mute(&cluster).await?;
            CommandOutput {
                message: Some("Notifications muted for this cluster".to_owned()),
                data: Some(json!({ "cluster": cluster, "muted": true, "changed": changed })),
            }
        }
        Command::NotificationsUnmuteCluster { cluster } => {
            let changed = service.unmute(&cluster).await?;
            CommandOutput {
                message: Some("Notifications unmuted for this cluster".to_owned()),
                data: Some(json!({ "cluster": cluster, "muted": false, "changed": changed })),
            }
        }
        other => {
            return Err(OxiError::internal(format!(
                "{} is not a notification centre command",
                other.id()
            )));
        }
    };
    Ok(output)
}
