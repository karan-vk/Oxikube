//! `helm::ShowValues`, `helm::ShowManifest` and `helm::RevealValues` on the command bus (tool
//! stubs `helm.show_values`, `helm.show_manifest`, `helm.reveal_values`). `helm::OpenReleases`
//! opens the page and is registered by `oxikube_helm_ui::releases`.
//!
//! All three are read-only. The two `Show` commands always answer masked text, whatever the user
//! revealed in the view; `RevealValues` is the user's switch for the view and answers with a
//! count, never a value.

use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::helm::ReleaseRef;
use oxikube_domain::{OxiError, OxiResult};
use serde_json::json;

use super::service::{Disclosure, ReleasesService};
use crate::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};

/// The commands [`register_commands`] installs.
pub const COMMANDS: [CommandId; 3] = [
    CommandId::HELM_SHOW_VALUES,
    CommandId::HELM_SHOW_MANIFEST,
    CommandId::HELM_REVEAL_VALUES,
];

/// Registers the three commands on `registry`; the binary installs it as
/// `oxikube_app::helm_releases`.
///
/// # Errors
///
/// A [`RegisterError`] when an id is already taken.
///
/// # Panics
///
/// When a command id is not declared in `oxikube_domain::command` (a programming error).
pub fn register_commands(
    registry: &mut CommandRegistry,
    service: ReleasesService,
) -> Result<(), RegisterError> {
    for id in COMMANDS {
        let meta = *command::lookup(id).expect("declared in oxikube_domain::command");
        let service = service.clone();
        registry.register(meta, move |command: Command, _: HandlerContext| {
            let service = service.clone();
            async move { run(&service, command).await }
        })?;
    }
    Ok(())
}

pub(super) async fn run(service: &ReleasesService, command: Command) -> OxiResult<CommandOutput> {
    match command {
        Command::HelmShowValues {
            cluster,
            namespace,
            name,
            revision,
            all,
        } => {
            let release = ReleaseRef::new(cluster, namespace, name);
            let view = service
                .values(&release, revision, all, Disclosure::Masked)
                .await?;
            Ok(CommandOutput {
                message: None,
                data: Some(json!({
                    "release": release.to_string(),
                    "revision": revision,
                    "all": all,
                    "values": &*view.text,
                    "masked": view.sensitive,
                })),
            })
        }
        Command::HelmShowManifest {
            cluster,
            namespace,
            name,
            revision,
        } => {
            let release = ReleaseRef::new(cluster, namespace, name);
            let view = service
                .manifest(&release, revision, Disclosure::Masked)
                .await?;
            Ok(CommandOutput {
                message: None,
                data: Some(json!({
                    "release": release.to_string(),
                    "revision": revision,
                    "manifest": &*view.text,
                    "secrets_masked": view.masked,
                })),
            })
        }
        Command::HelmRevealValues {
            cluster,
            namespace,
            name,
            reveal,
        } => {
            let release = ReleaseRef::new(cluster, namespace, name);
            let count = service.set_revealed(&release, reveal).await?;
            let message = if reveal {
                format!("Showing {count} masked values of {release} in the release view")
            } else {
                format!("Hiding the masked values of {release} again")
            };
            Ok(CommandOutput {
                message: Some(message),
                data: Some(
                    json!({ "release": release.to_string(), "revealed": reveal, "values": count }),
                ),
            })
        }
        other => Err(OxiError::internal(format!(
            "{} is not a Helm releases command",
            other.id()
        ))),
    }
}
