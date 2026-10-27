//! `helm::RepoAdd`, `RepoRemove` and `RepoUpdate` on the command bus (each with its MCP tool
//! stub: `helm.repo_add`, `helm.repo_remove`, `helm.repo_update`).
//!
//! They change Helm's local repository file, not a cluster, so none goes through a
//! `MutationGuard` tier. `helm::RepoAdd` fetches from an arbitrary URL (and `helm::RepoRemove` deletes the user's choice), so when an
//! *agent* asks, [`RepoPermission`] decides first (default [`DenyAgentRepoAdd`]: nobody until the agent epic's
//! permission prompt exists). `helm::ShowRepos` opens the page and is registered by
//! `oxikube_settings_ui`.

use std::sync::Arc;

use async_trait::async_trait;
use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::helm::RepoAddRequest;
use oxikube_domain::safety::Initiator;
use oxikube_domain::{OxiError, OxiResult};
use serde_json::json;

use super::service::HelmReposService;
use crate::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};

/// The commands [`register_commands`] installs.
pub const COMMANDS: [CommandId; 3] = [
    CommandId::HELM_REPO_ADD,
    CommandId::HELM_REPO_REMOVE,
    CommandId::HELM_REPO_UPDATE,
];

/// Who may add or remove a repository on an agent's behalf. (Refreshing indexes is open: it
/// only re-reads repositories the user already chose.)
#[async_trait]
pub trait RepoPermission: Send + Sync {
    /// Whether an agent may run `command` (`helm::RepoAdd` or `helm::RepoRemove`).
    async fn allows_agent(&self, command: &Command) -> bool;
}

/// The default policy: no agent may add or remove a repository.
#[derive(Debug, Clone, Copy, Default)]
pub struct DenyAgentRepoAdd;

#[async_trait]
impl RepoPermission for DenyAgentRepoAdd {
    async fn allows_agent(&self, _: &Command) -> bool {
        false
    }
}

/// Registers the three commands on `registry`; the binary installs it as `oxikube_app::helm_repos`.
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
    service: HelmReposService,
    permission: Arc<dyn RepoPermission>,
) -> Result<(), RegisterError> {
    for id in COMMANDS {
        let meta = *command::lookup(id).expect("declared in oxikube_domain::command");
        let (service, permission) = (service.clone(), permission.clone());
        registry.register(meta, move |command: Command, ctx: HandlerContext| {
            let (service, permission) = (service.clone(), permission.clone());
            async move { run(&service, &*permission, ctx.initiator(), command).await }
        })?;
    }
    Ok(())
}

pub(super) async fn run(
    service: &HelmReposService,
    permission: &dyn RepoPermission,
    initiator: Initiator,
    command: Command,
) -> OxiResult<CommandOutput> {
    let agent = initiator == Initiator::Agent;
    if agent
        && matches!(
            command,
            Command::HelmRepoAdd { .. } | Command::HelmRepoRemove { .. }
        )
        && !permission.allows_agent(&command).await
    {
        return Err(OxiError::forbidden(format!(
            "{} needs the user's permission",
            command.id()
        )));
    }
    match command {
        Command::HelmRepoAdd {
            name,
            url,
            username,
            insecure_skip_tls_verify,
            ca_file,
        } => {
            let mut request = RepoAddRequest::new(name.trim(), url.trim());
            request.username = username.filter(|u| !u.is_empty());
            request.insecure_skip_tls_verify = insecure_skip_tls_verify;
            request.ca_file = ca_file.filter(|c| !c.is_empty());
            let name = request.name.clone();
            service.add_with(request, !agent).await?;
            Ok(CommandOutput {
                message: Some(format!("Added repository {name}")),
                data: Some(json!({ "name": name })),
            })
        }
        Command::HelmRepoRemove { name } => {
            service.remove(&name).await?;
            Ok(CommandOutput {
                message: Some(format!("Removed repository {name}")),
                data: Some(json!({ "name": name })),
            })
        }
        Command::HelmRepoUpdate { names } => {
            let report = service.update(&names).await?;
            let failed: Vec<_> = report
                .failed
                .iter()
                .map(|(name, why)| json!({ "name": name, "error": why }))
                .collect();
            Ok(CommandOutput {
                message: Some(report.summary()),
                data: Some(json!({ "updated": report.updated, "failed": failed })),
            })
        }
        other => Err(OxiError::internal(format!(
            "{} is not a Helm repository command",
            other.id()
        ))),
    }
}
