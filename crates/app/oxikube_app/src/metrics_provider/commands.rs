//! `cluster::RedetectMetrics` and `cluster::ToggleMetrics` on the command bus (E13-S05), each
//! with its MCP tool stub (`app.cluster_redetect_metrics`, `app.cluster_toggle_metrics`).
//!
//! Neither changes anything in a cluster, so neither goes through a guard tier: the first lists
//! Services and sends one probe query, the second edits the user's own settings. Both therefore
//! work on a read-only cluster. `cluster::ShowMetricsSettings` opens a page, so the crate that
//! owns the page registers it (`oxikube_settings_ui`).

use std::sync::Arc;

use futures::future::BoxFuture;
use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::ids::ClusterId;
use oxikube_domain::{OxiError, OxiResult};
use serde_json::json;

use super::service::MetricsProviderService;
use crate::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};

/// Writes `prometheus.enabled` to the user's settings.
///
/// The app crate has no `gpui` and cannot see the settings store, so the binary implements this
/// on top of `oxikube_settings::ClusterSettings::set_metrics_enabled` (comment-preserving, hot
/// reload). The future resolves once the value is in the store; the change then reaches the
/// service through [`MetricsProviderService::settings_changed`].
pub trait MetricsSettingsWriter: Send + Sync {
    /// Sets `clusters.<cluster>.prometheus.enabled`. `name_hint` (the context name) names a new
    /// block so the user can recognise the opaque id in the file.
    fn set_enabled(
        &self,
        cluster: &ClusterId,
        name_hint: Option<&str>,
        enabled: bool,
    ) -> BoxFuture<'static, OxiResult<()>>;
}

/// Registers the two commands on `registry`. The binary installs it as
/// `oxikube_app::metrics_provider`.
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
    service: MetricsProviderService,
    writer: Arc<dyn MetricsSettingsWriter>,
) -> Result<(), RegisterError> {
    let commands = Arc::new(Commands { service, writer });
    for id in [
        CommandId::CLUSTER_REDETECT_METRICS,
        CommandId::CLUSTER_TOGGLE_METRICS,
    ] {
        let commands = commands.clone();
        let meta = *command::lookup(id).expect("declared in oxikube_domain::command");
        registry.register(meta, move |command: Command, _: HandlerContext| {
            let commands = commands.clone();
            async move { commands.run(command).await }
        })?;
    }
    Ok(())
}

struct Commands {
    service: MetricsProviderService,
    writer: Arc<dyn MetricsSettingsWriter>,
}

impl Commands {
    async fn run(&self, command: Command) -> OxiResult<CommandOutput> {
        match command {
            Command::ClusterRedetectMetrics { cluster } => {
                let status = self.service.redetect(&cluster)?;
                Ok(CommandOutput {
                    message: Some(status.headline()),
                    data: Some(json!({
                        "cluster": cluster,
                        "mode": status.mode.id(),
                        "detecting": status.is_detecting(),
                        "provider": status.provider().map(|p| p.summary()),
                        "unavailable": status.unavailable_text(),
                    })),
                })
            }
            Command::ClusterToggleMetrics { cluster, enabled } => {
                let before = self.service.metrics_enabled(&cluster);
                let enabled = enabled.unwrap_or(!before);
                let hint = self.service.context_name(&cluster);
                self.writer
                    .set_enabled(&cluster, hint.as_deref(), enabled)
                    .await?;
                // The settings echo reaches the service through `settings_changed`; calling it
                // here makes the command correct on its own and is a no-op when the echo came
                // first.
                self.service.settings_changed();
                Ok(CommandOutput {
                    message: Some(if enabled {
                        "History metrics are on for this cluster".to_owned()
                    } else {
                        "History metrics are off for this cluster".to_owned()
                    }),
                    data: Some(json!({ "cluster": cluster, "enabled": enabled })),
                })
            }
            other => Err(OxiError::internal(format!(
                "{} is not a metrics provider command",
                other.id()
            ))),
        }
    }
}
