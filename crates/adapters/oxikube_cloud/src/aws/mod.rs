//! [`AwsDiscovery`]: EKS discovery through the `aws` CLI (v2).
//!
//! Everything goes through `aws ... --output json` (the runner appends the flags) except the
//! calls that have no JSON form (`--version`, `configure list-profiles`, `configure get
//! region`), which print plain lines; no `~/.aws/credentials` value is ever read, and credentials never
//! appear on a command line.
//!
//! | Module | Holds |
//! |---|---|
//! | `detect` | `aws --version` (v2 required), per-profile `sts get-caller-identity` login check |
//! | `scopes` | profiles and regions: one [`CloudScope::Aws`] per profile and region to scan |
//! | `clusters` | `eks list-clusters` + parallel `eks describe-cluster`, and the JSON parsers |
//! | `entry` | the exec-auth [`KubeconfigEntry`] (`aws eks get-token`) |

mod clusters;
mod detect;
mod entry;
mod scopes;
#[cfg(test)]
mod tests;

use std::sync::Arc;

use async_trait::async_trait;
use oxikube_domain::OxiResult;
use oxikube_ports::{
    CloudDetection, CloudDiscoveryPort, CloudProvider, CloudScope, DiscoveredCluster,
    KubeconfigEntry,
};

use crate::process::ProcessRunner;
use crate::shared::Shared;

/// The profile every AWS CLI call uses when none is named.
pub const DEFAULT_PROFILE: &str = "default";

/// The `--profile P` flags for `profile`, or none for the implicit `default`: naming it
/// explicitly fails with `ProfileNotFound` when only environment credentials exist.
pub(crate) fn profile_args(profile: &str) -> Vec<String> {
    if profile == DEFAULT_PROFILE {
        Vec::new()
    } else {
        vec!["--profile".to_owned(), profile.to_owned()]
    }
}

/// What the user narrows the scan to (the `cloud.aws` settings, E18-S07).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AwsSettings {
    /// Profiles to scan; empty scans every profile `aws configure list-profiles` reports.
    pub profiles: Vec<String>,
    /// Regions to scan in every profile, besides each profile's own default region. AWS
    /// has no cheap "all regions" call, so none are scanned unless named here or defaulted.
    pub regions: Vec<String>,
}

/// EKS discovery. Cheap to clone-share behind an `Arc<dyn CloudDiscoveryPort>`.
pub struct AwsDiscovery {
    runner: Arc<dyn ProcessRunner>,
    settings: Shared<AwsSettings>,
}

impl AwsDiscovery {
    /// Discovery running `aws` through `runner`, narrowed by `settings`.
    pub fn new(runner: Arc<dyn ProcessRunner>, settings: AwsSettings) -> Self {
        Self {
            runner,
            settings: Shared::new(settings),
        }
    }

    /// Replaces the narrowing (the `cloud.aws` settings changed). The next scan uses it.
    pub fn set_settings(&self, settings: AwsSettings) {
        self.settings.set(settings);
    }
}

#[async_trait]
impl CloudDiscoveryPort for AwsDiscovery {
    fn id(&self) -> CloudProvider {
        CloudProvider::Aws
    }

    async fn detect(&self) -> OxiResult<CloudDetection> {
        self.detect_cli().await
    }

    async fn scopes(&self) -> OxiResult<Vec<CloudScope>> {
        self.scan_scopes().await
    }

    async fn list_clusters(&self, scope: &CloudScope) -> OxiResult<Vec<DiscoveredCluster>> {
        self.list_eks(scope).await
    }

    async fn kubeconfig_entry(&self, cluster: &DiscoveredCluster) -> OxiResult<KubeconfigEntry> {
        entry::kubeconfig_entry(cluster)
    }
}
