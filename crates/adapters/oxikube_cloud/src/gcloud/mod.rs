//! [`GcloudDiscovery`]: GKE discovery through the `gcloud` CLI (E18-S03).
//!
//! - **Detect**: `gcloud version`, the active account of `gcloud auth list` and whether
//!   Application Default Credentials exist (a file-exists check; the file is never read),
//!   plus whether `gke-gcloud-auth-plugin` is on the resolved `PATH`.
//! - **Scopes**: `gcloud projects list`, the `gcloud config` project first.
//! - **List**: `gcloud container clusters list --project P`, which spans every location of a
//!   project. A project whose Container API is disabled gets an error saying how to enable it.
//! - **Entry**: an exec-auth context running `gke-gcloud-auth-plugin`, named
//!   `gke:<project>:<location>:<name>`. No token is ever written; a missing plugin is an
//!   `Unsupported` error naming `gcloud components install gke-gcloud-auth-plugin`.
//!
//! Nothing here runs `gcloud auth print-access-token` or any command that prints a
//! credential, and the commands carry project ids and cluster names only.
//!
//! | Module | Holds |
//! |---|---|
//! | `detect` | the CLI, login, ADC and plugin checks, and the project listing |
//! | `parse` | serde mirrors of the CLI's JSON and their mapping onto domain types |
//! | `entry` | building the kubeconfig entry |

mod detect;
mod entry;
mod parse;
#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::cloud::{
    CloudDetection, CloudDiscoveryPort, CloudProvider, CloudScope, DiscoveredCluster,
    KubeconfigEntry,
};

use crate::process::{Cli, CliError, CliInvocation, ProcessRunner, RunOpts};
use crate::shared::Shared;

/// The plugin GKE contexts authenticate through.
pub(crate) const PLUGIN_COMMAND: &str = "gke-gcloud-auth-plugin";
/// What installs it.
pub(crate) const PLUGIN_INSTALL: &str = "gcloud components install gke-gcloud-auth-plugin";

/// Listing a project can take a while on a large organisation.
const LIST_TIMEOUT: Duration = Duration::from_secs(60);

/// The GKE [`CloudDiscoveryPort`] adapter.
#[derive(Clone)]
pub struct GcloudDiscovery {
    runner: Arc<dyn ProcessRunner>,
    projects: Shared<Vec<String>>,
}

impl GcloudDiscovery {
    /// An adapter running `gcloud` through `runner`.
    pub fn new(runner: Arc<dyn ProcessRunner>) -> Self {
        Self {
            runner,
            projects: Shared::new(Vec::new()),
        }
    }

    /// Scans only `projects` (the `cloud.gcloud.projects` setting); empty scans every project
    /// `gcloud projects list` reports. Naming projects skips that slow call. The next scan uses
    /// the new list.
    pub fn set_projects(&self, projects: Vec<String>) {
        self.projects.set(projects);
    }

    /// `gcloud args` asking for JSON, with a generous timeout.
    pub(super) fn json<I, S>(&self, args: I) -> CliInvocation
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        CliInvocation::new(Cli::Gcloud, args)
            .with_opts(RunOpts {
                timeout: Some(LIST_TIMEOUT),
                ..RunOpts::default()
            })
            .json()
    }
}

#[async_trait]
impl CloudDiscoveryPort for GcloudDiscovery {
    fn id(&self) -> CloudProvider {
        CloudProvider::Gcp
    }

    async fn detect(&self) -> OxiResult<CloudDetection> {
        self.detect_gcloud().await
    }

    async fn scopes(&self) -> OxiResult<Vec<CloudScope>> {
        self.list_projects().await
    }

    async fn list_clusters(&self, scope: &CloudScope) -> OxiResult<Vec<DiscoveredCluster>> {
        let project = project(scope)?;
        let args = ["container", "clusters", "list", "--project", project];
        let listed: Vec<parse::GkeCluster> = self
            .runner
            .run_json(self.json(args))
            .await
            .map_err(|error| map_gcloud_error(error, project))?;
        Ok(listed
            .into_iter()
            .filter_map(|cluster| cluster.into_discovered(scope))
            .collect())
    }

    async fn kubeconfig_entry(&self, cluster: &DiscoveredCluster) -> OxiResult<KubeconfigEntry> {
        self.build_entry(cluster).await
    }
}

/// The project of a GCP scope, or a `Validation` error for another provider's.
pub(crate) fn project(scope: &CloudScope) -> OxiResult<&str> {
    match scope {
        CloudScope::Gcp { project } => {
            if project.is_empty() || project.starts_with('-') {
                return Err(OxiError::validation(format!(
                    "`{project}` is not a Google Cloud project id"
                )));
            }
            Ok(project)
        }
        CloudScope::Aws { .. } | CloudScope::Azure { .. } => Err(OxiError::validation(format!(
            "{} scope `{scope}` given to the GKE adapter",
            scope.provider()
        ))),
    }
}

/// Maps a failed `gcloud container` call onto an `OxiError`; a disabled Container API gets
/// the command that enables it.
pub(crate) fn map_gcloud_error(error: CliError, project: &str) -> OxiError {
    let disabled = error.says("container.googleapis.com")
        && (error.says("has not been used")
            || error.says("is disabled")
            || error.says("service_disabled"));
    if disabled {
        return OxiError::unsupported(format!(
            "the Kubernetes Engine API is not enabled in project `{project}`: run \
             `gcloud services enable container.googleapis.com --project {project}`"
        ));
    }
    OxiError::from(error)
}
