//! The JSON `az` prints (`account list`, `version`, `aks list`, `aks show`) and how it maps
//! onto the domain types. Only the fields Oxikube uses are read, so new CLI fields are
//! harmless.

use oxikube_domain::cloud::{ClusterStatus, DiscoveredCluster};
use oxikube_ports::CloudScope;
use serde::Deserialize;

/// `az version -o json`.
#[derive(Debug, Deserialize)]
pub(super) struct AzVersion {
    #[serde(rename = "azure-cli")]
    pub azure_cli: Option<String>,
}

/// One entry of `az account list -o json`.
#[derive(Debug, Deserialize)]
pub(super) struct Subscription {
    id: String,
    name: Option<String>,
    state: Option<String>,
}

impl Subscription {
    /// Disabled, warned and deleted subscriptions cannot list clusters. A missing state is
    /// kept: older CLIs omit it.
    pub(super) fn is_enabled(&self) -> bool {
        self.state
            .as_deref()
            .is_none_or(|state| state.eq_ignore_ascii_case("enabled"))
    }

    pub(super) fn into_scope(self) -> CloudScope {
        CloudScope::Azure {
            subscription_id: self.id,
            subscription_name: self.name.filter(|name| !name.is_empty()),
        }
    }
}

/// `powerState` of a cluster.
#[derive(Debug, Deserialize)]
struct PowerState {
    code: Option<String>,
}

/// `aadProfile` of a cluster that has Azure AD sign-in (`null` otherwise).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AadProfile {
    pub tenant_id: Option<String>,
    /// Set by the legacy (non-managed) integration; managed AAD uses a well-known id.
    pub server_app_id: Option<String>,
}

/// One cluster of `az aks list -o json` or `az aks show -o json`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AksCluster {
    id: Option<String>,
    name: String,
    resource_group: Option<String>,
    location: String,
    fqdn: Option<String>,
    private_fqdn: Option<String>,
    kubernetes_version: Option<String>,
    current_kubernetes_version: Option<String>,
    provisioning_state: Option<String>,
    power_state: Option<PowerState>,
    pub aad_profile: Option<AadProfile>,
}

impl AksCluster {
    /// `true` when the cluster is stopped (`powerState.code == Stopped`).
    pub(super) fn is_stopped(&self) -> bool {
        self.status() == ClusterStatus::Stopped
    }

    /// Normalises provisioning state and power state. A cluster that is not in `Succeeded`
    /// reports that state; a succeeded one reports its power state.
    fn status(&self) -> ClusterStatus {
        let provisioning = self.provisioning_state.as_deref().unwrap_or("Succeeded");
        match provisioning.to_ascii_lowercase().as_str() {
            "succeeded" => {}
            "creating" => return ClusterStatus::Creating,
            "updating" | "upgrading" | "scaling" => return ClusterStatus::Updating,
            "deleting" => return ClusterStatus::Deleting,
            "failed" | "canceled" => return ClusterStatus::Failed,
            _ => return ClusterStatus::Other(provisioning.to_owned()),
        }
        let power = self.power_state.as_ref().and_then(|p| p.code.as_deref());
        match power.map(str::to_ascii_lowercase).as_deref() {
            None | Some("running") => ClusterStatus::Running,
            Some("stopped") => ClusterStatus::Stopped,
            Some(_) => ClusterStatus::Other(power.unwrap_or_default().to_owned()),
        }
    }

    pub(super) fn into_discovered(self, scope: &CloudScope) -> DiscoveredCluster {
        let status = self.status();
        let endpoint = self
            .fqdn
            .or(self.private_fqdn)
            .filter(|host| !host.is_empty())
            .map(|host| format!("https://{host}:443"));
        // The resource id carries the resource group the entry needs; rebuild it when the CLI
        // reported only the group.
        let native_id = self.id.or_else(|| match (scope, &self.resource_group) {
            (
                CloudScope::Azure {
                    subscription_id, ..
                },
                Some(group),
            ) => Some(format!(
                "/subscriptions/{subscription_id}/resourceGroups/{group}/providers/\
                 Microsoft.ContainerService/managedClusters/{}",
                self.name
            )),
            _ => None,
        });
        DiscoveredCluster {
            native_id,
            endpoint,
            kubernetes_version: self.current_kubernetes_version.or(self.kubernetes_version),
            status,
            ..DiscoveredCluster::new(scope.clone(), self.name, self.location)
        }
    }
}

/// The resource group of an AKS resource id
/// (`/subscriptions/S/resourceGroups/G/providers/Microsoft.ContainerService/managedClusters/N`).
pub(super) fn resource_group_of(native_id: &str) -> Option<&str> {
    let mut parts = native_id.split('/');
    parts.find(|part| part.eq_ignore_ascii_case("resourceGroups"))?;
    parts.next().filter(|group| !group.is_empty())
}
