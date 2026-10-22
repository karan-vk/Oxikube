//! The `helm::Rollback` and `helm::Uninstall` handlers.

use std::sync::Arc;

use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::helm::{ReleaseRef, RollbackRequest, UninstallRequest};
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::HelmPort;
use serde_json::json;

use crate::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};

/// Registers `helm::Rollback` and `helm::Uninstall` on `registry` (tool stubs `helm.rollback`
/// and `helm.uninstall`), running them through `helm`. The binary installs it as
/// `oxikube_app::helm_ops`.
///
/// The guard runs first: refused on a read-only cluster, a simple confirmation for the
/// rollback, the release name typed for the uninstall, an audit record naming the release and
/// the revision or flag. A dry-run dispatch asks `helm` for its `--dry-run` and changes nothing.
/// Output `data` is `{"outcome": "rolled_back" | "uninstalled" | "dry_run", ...}`.
///
/// # Errors
///
/// A [`RegisterError`] when either command is registered already.
pub fn register_commands(
    registry: &mut CommandRegistry,
    helm: Arc<dyn HelmPort>,
) -> Result<(), RegisterError> {
    for id in [CommandId::HELM_ROLLBACK, CommandId::HELM_UNINSTALL] {
        let meta = *command::lookup(id).ok_or(RegisterError::Undeclared(id))?;
        let helm = helm.clone();
        registry.register(meta, move |command: Command, cx: HandlerContext| {
            let helm = helm.clone();
            async move {
                let dry_run = cx.require_mutation()?.dry_run();
                run(helm.as_ref(), &command, dry_run).await
            }
        })?;
    }
    Ok(())
}

async fn run(helm: &dyn HelmPort, command: &Command, dry_run: bool) -> OxiResult<CommandOutput> {
    match command {
        Command::HelmRollback {
            cluster,
            namespace,
            release,
            revision,
        } => {
            let reference = ReleaseRef::new(cluster.clone(), namespace, release);
            let mut request = RollbackRequest::new(reference, *revision);
            request.dry_run = dry_run;
            helm.rollback(&request).await?;
            let (outcome, message) = if dry_run {
                (
                    "dry_run",
                    format!("Dry run: {release} would roll back to revision {revision}"),
                )
            } else {
                (
                    "rolled_back",
                    format!("Rolled {release} back to revision {revision}"),
                )
            };
            Ok(CommandOutput {
                message: Some(message),
                data: Some(json!({
                    "outcome": outcome,
                    "release": release,
                    "namespace": namespace,
                    "revision": revision,
                })),
            })
        }
        Command::HelmUninstall {
            cluster,
            namespace,
            release,
            keep_history,
        } => {
            let reference = ReleaseRef::new(cluster.clone(), namespace, release);
            let mut request = UninstallRequest::new(reference);
            request.keep_history = *keep_history;
            request.dry_run = dry_run;
            helm.uninstall(&request).await?;
            let (outcome, message) = if dry_run {
                (
                    "dry_run",
                    format!("Dry run: {release} would be uninstalled"),
                )
            } else {
                ("uninstalled", format!("Uninstalled release {release}"))
            };
            Ok(CommandOutput {
                message: Some(message),
                data: Some(json!({
                    "outcome": outcome,
                    "release": release,
                    "namespace": namespace,
                    "keep_history": keep_history,
                })),
            })
        }
        _ => Err(OxiError::validation(
            "not a helm::Rollback or helm::Uninstall",
        )),
    }
}
