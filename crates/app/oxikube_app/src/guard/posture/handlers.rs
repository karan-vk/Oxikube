//! The handlers of the posture commands and their registration.

use std::sync::Arc;

use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::ids::ClusterId;
use oxikube_domain::{OxiError, OxiResult};
use serde_json::json;

use super::{Posture, PrefsPatch, SharedWriter, patch_of};
use crate::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};
use crate::guard::policy;
use crate::session::ClusterSessionManager;

/// Registers `cluster::ToggleReadOnly`, `cluster::SetColour` and `cluster::ApplyPreset`
/// (and their MCP tool stubs: the toggle is privileged and gets none, see
/// [`CommandMeta::privileged`](oxikube_domain::CommandMeta::privileged)).
///
/// `bins/oxikube` installs it under the name `oxikube_app::posture`, passing the session
/// manager and the settings-backed [`PrefsWriter`](super::PrefsWriter).
///
/// # Errors
///
/// A [`RegisterError`] when one of the ids is already taken.
pub fn register_commands(
    registry: &mut CommandRegistry,
    sessions: ClusterSessionManager,
    writer: SharedWriter,
) -> Result<(), RegisterError> {
    let service = Arc::new(Service { sessions, writer });
    for id in [
        CommandId::CLUSTER_TOGGLE_READ_ONLY,
        CommandId::CLUSTER_SET_COLOUR,
        CommandId::CLUSTER_APPLY_PRESET,
    ] {
        let service = service.clone();
        let meta = *command::lookup(id).expect("declared in oxikube_domain::command");
        registry.register(meta, move |command: Command, cx: HandlerContext| {
            let service = service.clone();
            async move { service.run(command, &cx).await }
        })?;
    }
    Ok(())
}

struct Service {
    sessions: ClusterSessionManager,
    writer: SharedWriter,
}

impl Service {
    async fn run(&self, command: Command, cx: &HandlerContext) -> OxiResult<CommandOutput> {
        let not_posture =
            || OxiError::internal(format!("{} is not a posture command", command.id()));
        let cluster = policy::cluster_of(&command)
            .ok_or_else(not_posture)?
            .clone();
        let read_only = Posture::of(&self.sessions, &cluster).read_only;
        let patch = patch_of(&command, read_only).ok_or_else(not_posture)?;
        if cx.dry_run() {
            // A preview: report what would be true afterwards; write and change nothing.
            let before = Posture::of(&self.sessions, &cluster);
            let read_only = patch.read_only.unwrap_or(before.read_only);
            let colour = patch.colour.unwrap_or(before.colour);
            return Ok(CommandOutput {
                message: Some(format!(
                    "Dry run: {}",
                    message(&command, &before.label(&cluster))
                )),
                data: Some(json!({
                    "cluster": cluster,
                    "read_only": read_only,
                    "colour": colour,
                    "dry_run": true,
                })),
            });
        }
        tracing::info!(command = %command.id(), initiator = %cx.initiator(), "posture change");
        self.apply(&cluster, patch).await?;
        let now = Posture::of(&self.sessions, &cluster);
        Ok(CommandOutput {
            message: Some(message(&command, &now.label(&cluster))),
            data: Some(json!({
                "cluster": cluster,
                "read_only": now.read_only,
                "colour": now.colour,
            })),
        })
    }

    /// Persists `patch` and applies it to the live session; the order depends on direction.
    async fn apply(&self, cluster: &ClusterId, patch: PrefsPatch) -> OxiResult<()> {
        let before = Posture::of(&self.sessions, cluster);
        let name_hint = before.context.as_ref().map(|c| c.as_str().to_owned());
        let raises = patch.read_only == Some(true) && !before.read_only;

        // Raising protection takes effect at once; saving it is second. A failed save then
        // leaves the cluster read-only for this session and says so.
        if raises {
            self.live_read_only(cluster, true);
        }
        let saved = self
            .writer
            .write(cluster, name_hint.as_deref(), patch)
            .await;
        if let Err(err) = saved {
            return Err(if raises {
                OxiError::new(
                    err.kind(),
                    format!(
                        "read-only mode is on for this session but could not be saved: {}",
                        err.message()
                    ),
                )
            } else {
                err
            });
        }
        // The settings echo reaches the session through `set_prefs_table`; applying the same
        // values here makes the command correct on its own (and a no-op when the echo came
        // first).
        if let Some(read_only) = patch.read_only {
            self.live_read_only(cluster, read_only);
        }
        if let Some(colour) = patch.colour {
            let _ = self.sessions.set_colour(cluster, colour);
        }
        Ok(())
    }

    fn live_read_only(&self, cluster: &ClusterId, read_only: bool) {
        // `NotFound` means no session is open: the setting alone decides at the next open.
        let _ = self.sessions.set_read_only(cluster, read_only);
    }
}

fn message(command: &Command, label: &str) -> String {
    match command {
        Command::ClusterToggleReadOnly { .. } => format!("Updated read-only mode of {label}"),
        Command::ClusterSetColour {
            colour: Some(c), ..
        } => {
            format!("Set the colour of {label} to {c}")
        }
        Command::ClusterSetColour { colour: None, .. } => {
            format!("Cleared the colour of {label}")
        }
        Command::ClusterApplyPreset { preset, .. } => {
            format!("Applied the {} preset to {label}", preset.label())
        }
        _ => String::new(),
    }
}
