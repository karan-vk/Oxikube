//! Serde mirrors of `gcloud`'s JSON (unknown fields are ignored, so a newer CLI still parses)
//! and their mapping onto domain types.

use oxikube_ports::cloud::{CloudScope, ClusterStatus, DiscoveredCluster};
use serde::Deserialize;

/// One element of `gcloud container clusters list|describe --format=json`.
#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct GkeCluster {
    pub name: String,
    pub location: String,
    pub endpoint: Option<String>,
    pub current_master_version: Option<String>,
    pub status: Option<String>,
    pub self_link: Option<String>,
    pub master_auth: Option<MasterAuth>,
}

/// The `masterAuth` block; only the public CA is read (never the client key or password
/// fields older clusters may carry).
#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct MasterAuth {
    pub cluster_ca_certificate: Option<String>,
}

impl GkeCluster {
    /// The domain cluster for `scope`; `None` for an element without a name.
    pub(crate) fn into_discovered(self, scope: &CloudScope) -> Option<DiscoveredCluster> {
        if self.name.is_empty() {
            return None;
        }
        Some(DiscoveredCluster {
            native_id: self.self_link,
            endpoint: non_empty(self.endpoint).map(|ip| format!("https://{ip}")),
            ca_data: self
                .master_auth
                .and_then(|auth| non_empty(auth.cluster_ca_certificate)),
            kubernetes_version: non_empty(self.current_master_version),
            status: status(self.status.as_deref()),
            ..DiscoveredCluster::new(scope.clone(), self.name, self.location)
        })
    }
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|v| !v.is_empty())
}

/// GKE's `status` word onto [`ClusterStatus`].
fn status(word: Option<&str>) -> ClusterStatus {
    match word {
        Some("RUNNING") => ClusterStatus::Running,
        Some("PROVISIONING") => ClusterStatus::Creating,
        Some("RECONCILING") => ClusterStatus::Updating,
        Some("STOPPING") => ClusterStatus::Deleting,
        Some("ERROR" | "DEGRADED") => ClusterStatus::Failed,
        Some(other) => ClusterStatus::Other(other.to_owned()),
        None => ClusterStatus::Other("UNKNOWN".to_owned()),
    }
}

/// One element of `gcloud projects list --format=json`.
#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct Project {
    pub project_id: String,
    pub lifecycle_state: Option<String>,
}

impl Project {
    /// Active (not pending deletion) and identified.
    pub(crate) fn is_usable(&self) -> bool {
        !self.project_id.is_empty()
            && self
                .lifecycle_state
                .as_deref()
                .is_none_or(|state| state == "ACTIVE")
    }
}

/// One element of `gcloud auth list --format=json`.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Account {
    pub account: String,
    pub status: String,
}
