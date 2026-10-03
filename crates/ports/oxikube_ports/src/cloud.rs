//! [`CloudDiscoveryPort`]: finding clusters through the cloud providers' CLIs.
//!
//! # Adapter
//!
//! Implemented by `oxikube_cloud`, which drives the `aws`, `gcloud` and `az` CLIs the
//! user already has logged in. Oxikube never stores their credentials; discovered
//! clusters carry no secrets and are turned into contexts by `ClusterSourcePort`.
//! Calls spawn processes and run off the UI thread.

use async_trait::async_trait;
use oxikube_domain::OxiResult;

/// A supported cloud provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CloudProvider {
    /// Amazon EKS through the `aws` CLI.
    Aws,
    /// Google GKE through the `gcloud` CLI.
    Gcp,
    /// Azure AKS through the `az` CLI.
    Azure,
}

/// Whether a provider's CLI can be used right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CloudToolStatus {
    /// The CLI is not installed.
    NotInstalled,
    /// The CLI is installed but not logged in.
    NotAuthenticated,
    /// The CLI is installed and logged in.
    Ready,
}

/// A managed cluster found at a provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredCluster {
    /// Which provider it is at.
    pub provider: CloudProvider,
    /// The cluster's name at the provider.
    pub name: String,
    /// Region or location.
    pub region: Option<String>,
    /// Account, project or subscription it belongs to.
    pub account: Option<String>,
    /// API server endpoint, if the provider reports it.
    pub endpoint: Option<String>,
}

/// Discovers clusters from cloud CLIs.
#[async_trait]
pub trait CloudDiscoveryPort: Send + Sync {
    /// Whether `provider`'s CLI is installed and logged in.
    async fn tool_status(&self, provider: CloudProvider) -> OxiResult<CloudToolStatus>;

    /// The clusters visible to the logged-in identity of `provider`. Fails with
    /// `Auth` when not logged in and `Unsupported` when the CLI is missing.
    async fn discover(&self, provider: CloudProvider) -> OxiResult<Vec<DiscoveredCluster>>;
}
