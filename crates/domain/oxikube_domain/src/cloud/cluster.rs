//! [`DiscoveredCluster`]: a managed cluster as a provider's CLI reports it.

use std::fmt;

use serde::{Deserialize, Serialize};

use super::provider::CloudProvider;
use super::scope::CloudScope;

/// A managed cluster's lifecycle state, normalised across providers. Each adapter maps its
/// own words onto it (EKS `ACTIVE`, GKE `RUNNING` and AKS `Succeeded` + power state `Running`
/// are all [`Running`](Self::Running)); a word it does not know is kept in
/// [`Other`](Self::Other).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClusterStatus {
    /// Serving; connecting should work.
    Running,
    /// Being created.
    Creating,
    /// Being upgraded or reconfigured; usually still reachable.
    Updating,
    /// Being deleted.
    Deleting,
    /// Stopped or deallocated by its owner (AKS stop, GKE with no nodes).
    Stopped,
    /// The provider reports an error or a degraded control plane.
    Failed,
    /// A state this adapter does not know, as the provider spelled it.
    Other(String),
}

impl ClusterStatus {
    /// `true` when connecting is worth trying ([`Running`](Self::Running) or
    /// [`Updating`](Self::Updating)).
    #[must_use]
    pub fn is_connectable(&self) -> bool {
        matches!(self, Self::Running | Self::Updating)
    }
}

impl fmt::Display for ClusterStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Running => "Running",
            Self::Creating => "Creating",
            Self::Updating => "Updating",
            Self::Deleting => "Deleting",
            Self::Stopped => "Stopped",
            Self::Failed => "Failed",
            Self::Other(word) => word,
        })
    }
}

/// A managed cluster found at a provider by `CloudDiscoveryPort::list_clusters`.
///
/// Carries no credentials: the endpoint and the CA certificate are what any client of the
/// cluster sees, and connecting goes through the exec plugin of a
/// [`KubeconfigEntry`](super::KubeconfigEntry).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DiscoveredCluster {
    /// The listing it came from (and so its provider).
    pub scope: CloudScope,
    /// The cluster's name at the provider (`prod-eu`).
    pub name: String,
    /// Region or location (`eu-west-1`, `europe-west1-b`, `westeurope`). For AWS it equals
    /// the scope's region; for GCP and Azure the scope spans locations.
    pub location: String,
    /// The provider's own id: the EKS ARN, the GKE `selfLink`, the AKS resource id. `None`
    /// when the listing did not report one.
    pub native_id: Option<String>,
    /// API server URL (`https://...`), when the provider reported it.
    pub endpoint: Option<String>,
    /// The cluster CA certificate, base64 PEM as kubeconfig's `certificate-authority-data`
    /// holds it. Public data, not a credential.
    pub ca_data: Option<String>,
    /// The control plane's Kubernetes version (`1.30`, `1.30.5-gke.1014001`).
    pub kubernetes_version: Option<String>,
    /// Lifecycle state.
    pub status: ClusterStatus,
}

impl DiscoveredCluster {
    /// A running cluster with only the required fields set; fill the rest with struct update
    /// syntax.
    pub fn new(scope: CloudScope, name: impl Into<String>, location: impl Into<String>) -> Self {
        Self {
            scope,
            name: name.into(),
            location: location.into(),
            native_id: None,
            endpoint: None,
            ca_data: None,
            kubernetes_version: None,
            status: ClusterStatus::Running,
        }
    }

    /// The provider it is at.
    #[must_use]
    pub fn provider(&self) -> CloudProvider {
        self.scope.provider()
    }

    /// A stable key unique across providers and scopes (`aws/prod/eu-west-1/prod-eu`), for the
    /// cache, the managed kubeconfig and "already added" checks. Two clusters with the same
    /// name in different locations of one GCP project or Azure subscription stay apart.
    #[must_use]
    pub fn key(&self) -> String {
        format!("{}/{}/{}", self.scope.key(), self.location, self.name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_separate_locations_and_scopes() {
        let gcp = CloudScope::Gcp {
            project: "p".into(),
        };
        let a = DiscoveredCluster::new(gcp.clone(), "c", "europe-west1");
        let b = DiscoveredCluster::new(gcp, "c", "us-east1");
        assert_ne!(a.key(), b.key());
        assert_eq!(a.key(), "gcp/p/europe-west1/c");
        assert_eq!(a.provider(), CloudProvider::Gcp);
        assert_eq!(a.status, ClusterStatus::Running);
    }

    #[test]
    fn status_is_connectable_only_when_serving() {
        assert!(ClusterStatus::Running.is_connectable());
        assert!(ClusterStatus::Updating.is_connectable());
        for status in [
            ClusterStatus::Creating,
            ClusterStatus::Deleting,
            ClusterStatus::Stopped,
            ClusterStatus::Failed,
            ClusterStatus::Other("PENDING".into()),
        ] {
            assert!(!status.is_connectable(), "{status}");
        }
        assert_eq!(
            ClusterStatus::Other("PENDING".into()).to_string(),
            "PENDING"
        );
    }

    #[test]
    fn serde_round_trips_for_the_cache() {
        let cluster = DiscoveredCluster {
            endpoint: Some("https://ABC.gr7.eu-west-1.eks.amazonaws.com".into()),
            ca_data: Some("LS0tLS1CRUdJTi".into()),
            kubernetes_version: Some("1.30".into()),
            native_id: Some("arn:aws:eks:eu-west-1:000000000000:cluster/prod".into()),
            status: ClusterStatus::Other("PENDING".into()),
            ..DiscoveredCluster::new(
                CloudScope::Aws {
                    profile: "prod".into(),
                    region: "eu-west-1".into(),
                },
                "prod",
                "eu-west-1",
            )
        };
        let json = serde_json::to_string(&cluster).unwrap();
        let back: DiscoveredCluster = serde_json::from_str(&json).unwrap();
        assert_eq!(back, cluster);
    }
}
