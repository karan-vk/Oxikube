//! [`CloudScope`]: what one cluster listing covers.

use std::fmt;

use serde::{Deserialize, Serialize};

use super::provider::CloudProvider;

/// What one `list_clusters` call covers: the unit discovery fans out over, caches and filters
/// by. Each variant names the provider, so a scope can never be handed to the wrong adapter
/// unnoticed.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "provider", rename_all = "lowercase")]
pub enum CloudScope {
    /// One AWS named profile in one region (EKS lists clusters per region).
    Aws {
        /// The profile in `~/.aws/config` (`default`, `prod-admin`).
        profile: String,
        /// The region (`eu-west-1`).
        region: String,
    },
    /// One Google Cloud project (GKE lists every location of a project at once).
    Gcp {
        /// The project id (`my-project-123`).
        project: String,
    },
    /// One Azure subscription (AKS lists every resource group of a subscription at once).
    Azure {
        /// The subscription id (a UUID).
        subscription_id: String,
        /// Its display name, when the CLI reported one.
        subscription_name: Option<String>,
    },
}

impl CloudScope {
    /// The provider the scope belongs to.
    #[must_use]
    pub fn provider(&self) -> CloudProvider {
        match self {
            Self::Aws { .. } => CloudProvider::Aws,
            Self::Gcp { .. } => CloudProvider::Gcp,
            Self::Azure { .. } => CloudProvider::Azure,
        }
    }

    /// A stable string key (`aws/prod/eu-west-1`, `gcp/my-project`, `azure/<id>`) for caches
    /// and settings filters. Display names are left out, so renaming a subscription keeps it.
    #[must_use]
    pub fn key(&self) -> String {
        match self {
            Self::Aws { profile, region } => format!("aws/{profile}/{region}"),
            Self::Gcp { project } => format!("gcp/{project}"),
            Self::Azure {
                subscription_id, ..
            } => format!("azure/{subscription_id}"),
        }
    }
}

impl fmt::Display for CloudScope {
    /// For people: `prod / eu-west-1`, `my-project`, the subscription's name (else its id).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Aws { profile, region } => write!(f, "{profile} / {region}"),
            Self::Gcp { project } => f.write_str(project),
            Self::Azure {
                subscription_id,
                subscription_name,
            } => f.write_str(subscription_name.as_deref().unwrap_or(subscription_id)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn azure(name: Option<&str>) -> CloudScope {
        CloudScope::Azure {
            subscription_id: "00000000-0000-0000-0000-000000000001".into(),
            subscription_name: name.map(str::to_owned),
        }
    }

    #[test]
    fn keys_ignore_display_names_and_name_the_provider() {
        let aws = CloudScope::Aws {
            profile: "prod".into(),
            region: "eu-west-1".into(),
        };
        assert_eq!(aws.key(), "aws/prod/eu-west-1");
        assert_eq!(aws.to_string(), "prod / eu-west-1");
        assert_eq!(aws.provider(), CloudProvider::Aws);
        assert_eq!(azure(Some("Dev")).key(), azure(None).key());
        assert_eq!(azure(Some("Dev")).to_string(), "Dev");
        assert_eq!(
            azure(None).to_string(),
            "00000000-0000-0000-0000-000000000001"
        );
        let gcp = CloudScope::Gcp {
            project: "p-1".into(),
        };
        assert_eq!(
            (gcp.key(), gcp.provider()),
            ("gcp/p-1".into(), CloudProvider::Gcp)
        );
    }

    #[test]
    fn serde_round_trips_with_a_provider_tag() {
        let scope = azure(Some("Dev"));
        let json = serde_json::to_value(&scope).unwrap();
        assert_eq!(json["provider"], "azure");
        let back: CloudScope = serde_json::from_value(json).unwrap();
        assert_eq!(back, scope);
    }
}
