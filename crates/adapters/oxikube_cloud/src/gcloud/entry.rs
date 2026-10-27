//! The exec-auth kubeconfig entry for a GKE cluster.

use oxikube_domain::ids::ContextName;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::cloud::{
    CloudScope, DiscoveredCluster, ExecInteractiveMode, ExecPlugin, KubeconfigEntry,
};

use super::parse::GkeCluster;
use super::{GcloudDiscovery, PLUGIN_COMMAND, PLUGIN_INSTALL, map_gcloud_error};
use crate::process::Cli;

/// The deterministic context name `gke:<project>:<location>:<name>`.
pub(crate) fn context_name(project: &str, location: &str, name: &str) -> ContextName {
    ContextName::new(format!("gke:{project}:{location}:{name}"))
}

impl GcloudDiscovery {
    pub(super) async fn build_entry(
        &self,
        cluster: &DiscoveredCluster,
    ) -> OxiResult<KubeconfigEntry> {
        let project = self.project(&cluster.scope)?.to_owned();
        let Some(plugin) = self.runner.locate(Cli::GkeGcloudAuthPlugin).await else {
            return Err(OxiError::unsupported(format!(
                "`{PLUGIN_COMMAND}` is not installed, so GKE cluster `{}` cannot be connected: \
                 run `{PLUGIN_INSTALL}` or set `{}` in settings",
                cluster.name,
                Cli::GkeGcloudAuthPlugin.setting_key()
            )));
        };
        let mut found = cluster.clone();
        if found.endpoint.is_none() || found.ca_data.is_none() {
            found = self.describe(&found, &project).await?;
        }
        let endpoint = found.endpoint.clone().ok_or_else(|| {
            OxiError::internal(format!(
                "GKE cluster `{}` reported no endpoint (is it still being created?)",
                cluster.name
            ))
        })?;
        let mut exec = ExecPlugin::new(
            plugin.to_string_lossy().into_owned(),
            Vec::new(),
            ExecInteractiveMode::IfAvailable,
        );
        exec.provide_cluster_info = true;
        exec.install_hint = Some(format!(
            "{PLUGIN_COMMAND} is required to connect to GKE clusters; install it with \
             `{PLUGIN_INSTALL}`"
        ));
        Ok(KubeconfigEntry {
            context: context_name(&project, &found.location, &found.name),
            server: endpoint,
            ca_data: found.ca_data,
            exec,
        })
    }

    /// Fills the endpoint and CA the listing lacked from `clusters describe`.
    async fn describe(
        &self,
        cluster: &DiscoveredCluster,
        project: &str,
    ) -> OxiResult<DiscoveredCluster> {
        let args = [
            "container",
            "clusters",
            "describe",
            cluster.name.as_str(),
            "--location",
            cluster.location.as_str(),
            "--project",
            project,
        ];
        let described: GkeCluster = self
            .runner
            .run_json(self.json(args))
            .await
            .map_err(|error| map_gcloud_error(error, project))?;
        let scope: &CloudScope = &cluster.scope;
        let mut fresh = described
            .into_discovered(scope)
            .unwrap_or_else(|| cluster.clone());
        fresh.location.clone_from(&cluster.location);
        fresh.name.clone_from(&cluster.name);
        Ok(fresh)
    }
}
