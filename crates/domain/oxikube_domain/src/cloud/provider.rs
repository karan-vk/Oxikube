//! [`CloudProvider`] and what detecting its CLI found ([`CloudDetection`]).

use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// A supported cloud provider, and so the identity of one `CloudDiscoveryPort` adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CloudProvider {
    /// Amazon EKS through the `aws` CLI.
    Aws,
    /// Google GKE through the `gcloud` CLI.
    Gcp,
    /// Azure AKS through the `az` CLI.
    Azure,
}

impl CloudProvider {
    /// Every provider, in the order the catalog lists them.
    pub const ALL: [Self; 3] = [Self::Aws, Self::Gcp, Self::Azure];

    /// The stable lower-case id (`aws`, `gcp`, `azure`) used in settings, cache keys and the
    /// managed kubeconfig file names.
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::Aws => "aws",
            Self::Gcp => "gcp",
            Self::Azure => "azure",
        }
    }

    /// The vendor CLI the adapter drives (`aws`, `gcloud`, `az`).
    #[must_use]
    pub fn cli(self) -> &'static str {
        match self {
            Self::Aws => "aws",
            Self::Gcp => "gcloud",
            Self::Azure => "az",
        }
    }

    /// The managed Kubernetes service's name, for headings (`EKS`, `GKE`, `AKS`).
    #[must_use]
    pub fn service(self) -> &'static str {
        match self {
            Self::Aws => "EKS",
            Self::Gcp => "GKE",
            Self::Azure => "AKS",
        }
    }
}

impl fmt::Display for CloudProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Aws => "AWS",
            Self::Gcp => "Google Cloud",
            Self::Azure => "Azure",
        })
    }
}

/// Whether a provider's CLI can be used right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloudToolStatus {
    /// The CLI is not installed (not at the configured path, on the login-shell `PATH` or in
    /// the usual install folders). The catalog hides the provider.
    NotInstalled,
    /// The CLI is installed but has no usable login.
    NotAuthenticated,
    /// The CLI is installed and logged in.
    Ready,
}

/// One thing the user can do to make discovery or connecting work: log in, install an auth
/// plugin, upgrade the CLI. Shown next to the provider; never contains a credential.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SetupHint {
    /// What is wrong, as one sentence (`gke-gcloud-auth-plugin is not installed`).
    pub summary: String,
    /// The command that fixes it, when there is one
    /// (`gcloud components install gke-gcloud-auth-plugin`).
    pub command: Option<String>,
}

impl SetupHint {
    /// A hint with a fixing command.
    pub fn with_command(summary: impl Into<String>, command: impl Into<String>) -> Self {
        Self {
            summary: summary.into(),
            command: Some(command.into()),
        }
    }
}

/// What detecting a provider's CLI found: `CloudDiscoveryPort::detect`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudDetection {
    /// Which provider.
    pub provider: CloudProvider,
    /// Installed and logged in, or not.
    pub status: CloudToolStatus,
    /// Where the CLI is; `None` exactly when it is [`CloudToolStatus::NotInstalled`].
    pub binary: Option<PathBuf>,
    /// The version the CLI reported (`2.15.30`), when it ran.
    pub version: Option<String>,
    /// What to do about a missing login, a missing auth plugin or an old CLI. Empty when all
    /// is well.
    pub hints: Vec<SetupHint>,
}

impl CloudDetection {
    /// The CLI was not found: no binary, no version, one hint to install it.
    pub fn not_installed(provider: CloudProvider) -> Self {
        Self {
            provider,
            status: CloudToolStatus::NotInstalled,
            binary: None,
            version: None,
            hints: vec![SetupHint {
                summary: format!(
                    "the `{}` CLI was not found; install it or set its path in settings",
                    provider.cli()
                ),
                command: None,
            }],
        }
    }

    /// `true` when the provider can list clusters now.
    #[must_use]
    pub fn is_ready(&self) -> bool {
        self.status == CloudToolStatus::Ready
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_and_clis_are_stable() {
        let ids: Vec<_> = CloudProvider::ALL.iter().map(|p| p.id()).collect();
        assert_eq!(ids, ["aws", "gcp", "azure"]);
        let clis: Vec<_> = CloudProvider::ALL.iter().map(|p| p.cli()).collect();
        assert_eq!(clis, ["aws", "gcloud", "az"]);
        assert_eq!(CloudProvider::Gcp.service(), "GKE");
        assert_eq!(CloudProvider::Azure.to_string(), "Azure");
        assert_eq!(
            serde_json::to_string(&CloudProvider::Gcp).unwrap(),
            "\"gcp\""
        );
    }

    #[test]
    fn not_installed_has_no_binary_and_one_hint() {
        let detection = CloudDetection::not_installed(CloudProvider::Aws);
        assert_eq!(detection.status, CloudToolStatus::NotInstalled);
        assert!(detection.binary.is_none() && detection.version.is_none());
        assert!(!detection.is_ready());
        assert_eq!(detection.hints.len(), 1);
        assert!(detection.hints[0].summary.contains("`aws`"));
    }
}
