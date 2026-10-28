//! The `cloud::*` commands: the handlers a `CommandBus` registers (non-negotiable 4).
//!
//! The Cloud section's buttons, the palette and the MCP tools `app.cloud_refresh`,
//! `app.cloud_cancel`, `app.cloud_add` and `app.cloud_remove` all arrive as a [`Command`] and
//! run here. None of them changes a cluster or the user's own kubeconfig: refresh and cancel run
//! the provider CLIs read-only, add and remove edit Oxikube's managed kubeconfigs. So none goes
//! through `MutationGuard`.

use futures::StreamExt as _;
use std::sync::Arc;

use futures::stream;
use oxikube_domain::cloud::{CloudProvider, DiscoveredCluster};
use oxikube_domain::command::{self, CloudRefreshMode, Command, CommandId};
use oxikube_domain::{OxiError, OxiResult};
use serde_json::{Value, json};

use super::event::{CloudDiscoveryEvent, RefreshMode};
use super::service::CloudDiscoveryService;
use crate::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};

/// Registers the four `cloud::*` commands, with their tool stubs, over `service`.
///
/// Call it through [`CommandRegistry::install`] from the binary that owns the service.
///
/// # Errors
///
/// A [`RegisterError`] when one of the ids is registered already.
pub fn register_commands(
    registry: &mut CommandRegistry,
    service: &CloudDiscoveryService,
) -> Result<(), RegisterError> {
    for id in [
        CommandId::CLOUD_REFRESH,
        CommandId::CLOUD_CANCEL,
        CommandId::CLOUD_ADD,
        CommandId::CLOUD_REMOVE,
    ] {
        let meta = *command::lookup(id).ok_or(RegisterError::Undeclared(id))?;
        let service = service.clone();
        registry.register(meta, move |command: Command, _: HandlerContext| {
            let service = service.clone();
            async move { service.execute(&command).await }
        })?;
    }
    Ok(())
}

impl CloudDiscoveryService {
    /// Runs a `cloud::*` command. A refresh runs to its end (all providers at once) before it
    /// returns; every [`CloudDiscoveryEvent`] of it goes to the [`watch`](Self::watch) streams
    /// meanwhile, which is how the Cloud section shows progress.
    ///
    /// # Errors
    ///
    /// `Unsupported` for any other command; `NotFound` when `cloud::Add` names a cluster the last
    /// listing does not hold (refresh first) or `cloud::Remove` a context that was not added;
    /// the first provider's error when every refreshed provider failed; otherwise as
    /// [`add`](Self::add) and [`remove`](Self::remove).
    pub async fn execute(&self, command: &Command) -> OxiResult<CommandOutput> {
        match command {
            Command::CloudRefresh { provider, mode } => self.run_refresh(*provider, *mode).await,
            Command::CloudCancel { provider } => {
                let stopped = self
                    .targets(*provider)
                    .into_iter()
                    .filter(|provider| self.cancel(*provider))
                    .count();
                Ok(CommandOutput::message(match stopped {
                    0 => "No cloud discovery was running.".to_owned(),
                    n => format!(
                        "Stopped {n} cloud discover{}.",
                        if n == 1 { "y" } else { "ies" }
                    ),
                }))
            }
            Command::CloudAdd { provider, cluster } => self.run_add(*provider, cluster).await,
            Command::CloudRemove { provider, context } => {
                let removed = self.remove(*provider, context).await?;
                let tail = if removed.disconnected {
                    " and disconnected its open session"
                } else {
                    ""
                };
                Ok(CommandOutput {
                    message: Some(format!("Removed {}{tail}.", removed.context)),
                    data: Some(json!({
                        "context": removed.context.to_string(),
                        "disconnected": removed.disconnected,
                    })),
                })
            }
            other => Err(OxiError::unsupported(format!(
                "{} is not a cloud command",
                other.id()
            ))),
        }
    }

    /// The providers a command names (`None`: all that have an adapter).
    fn targets(&self, provider: Option<CloudProvider>) -> Vec<CloudProvider> {
        provider.map_or_else(|| self.providers(), |provider| vec![provider])
    }

    async fn run_refresh(
        &self,
        provider: Option<CloudProvider>,
        mode: CloudRefreshMode,
    ) -> OxiResult<CommandOutput> {
        let mode = match mode {
            CloudRefreshMode::IfStale => RefreshMode::IfStale,
            CloudRefreshMode::Force => RefreshMode::Force,
        };
        let targets = self.targets(provider);
        // Every stream is created (and so every run started) before the first is polled: the
        // providers list in parallel.
        let streams: Vec<_> = targets
            .iter()
            .map(|provider| self.refresh(*provider, None, mode))
            .collect();
        let mut events = stream::select_all(streams);
        let mut clusters = Vec::new();
        let mut first_error: Option<OxiError> = None;
        let (mut failed, mut cancelled) = (0, 0);
        while let Some(event) = events.next().await {
            match &event {
                CloudDiscoveryEvent::Finished {
                    clusters: found, ..
                } => {
                    clusters.extend(found.iter().cloned());
                }
                CloudDiscoveryEvent::Failed { error, .. } => {
                    failed += 1;
                    first_error.get_or_insert_with(|| OxiError::new(error.kind(), error.message()));
                }
                CloudDiscoveryEvent::Cancelled { .. } => cancelled += 1,
                _ => {}
            }
            self.publish(&Arc::new(event));
        }
        if let Some(error) = first_error.filter(|_| failed == targets.len()) {
            return Err(error);
        }
        Ok(refresh_output(&clusters, failed, cancelled))
    }

    async fn run_add(&self, provider: CloudProvider, key: &str) -> OxiResult<CommandOutput> {
        let found = self
            .cached(provider)
            .await
            .into_iter()
            .find(|cluster| cluster.key() == key);
        let Some(cluster) = found else {
            return Err(OxiError::not_found(format!(
                "{key} is not in the last {provider} listing; refresh the Cloud section first."
            )));
        };
        let added = self.add(&cluster).await?;
        let message = if added.unchanged {
            format!("{} was already in the catalog.", added.context)
        } else {
            format!("Added {} to the catalog.", added.context)
        };
        Ok(CommandOutput {
            message: Some(message),
            data: Some(json!({
                "context": added.context.to_string(),
                "unchanged": added.unchanged,
            })),
        })
    }
}

fn refresh_output(
    clusters: &[DiscoveredCluster],
    failed: usize,
    cancelled: usize,
) -> CommandOutput {
    let mut message = format!(
        "Found {} cloud cluster{}.",
        clusters.len(),
        if clusters.len() == 1 { "" } else { "s" }
    );
    if failed > 0 {
        message.push_str(&format!(" {failed} provider(s) failed."));
    }
    if cancelled > 0 {
        message.push_str(&format!(" {cancelled} provider(s) were cancelled."));
    }
    CommandOutput {
        message: Some(message),
        data: Some(json!({ "clusters": clusters.iter().map(cluster_json).collect::<Vec<_>>() })),
    }
}

/// A cluster as a tool result: names and versions, nothing secret.
fn cluster_json(cluster: &DiscoveredCluster) -> Value {
    json!({
        "key": cluster.key(),
        "provider": cluster.provider().id(),
        "name": cluster.name,
        "location": cluster.location,
        "version": cluster.kubernetes_version,
        "status": cluster.status.to_string(),
    })
}
