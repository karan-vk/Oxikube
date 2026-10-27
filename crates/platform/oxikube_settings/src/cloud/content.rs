//! What one settings layer says about cloud discovery, as written in `settings.json`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Which providers discovery runs. An unknown provider id is an error: the key is reported and
/// the previous value stays.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CloudProvidersContent {
    /// Discover Amazon EKS clusters through the `aws` CLI. Default `true`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aws: Option<bool>,
    /// Discover Google GKE clusters through the `gcloud` CLI. Default `true`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gcloud: Option<bool>,
    /// Discover Azure AKS clusters through the `az` CLI. Default `true`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub az: Option<bool>,
}

/// Narrowing the AWS scan.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CloudAwsContent {
    /// The AWS named profiles to scan. An empty list scans every profile `aws configure
    /// list-profiles` reports ("all").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profiles: Option<Vec<String>>,
    /// Regions to scan in every profile, besides each profile's own default region. An empty
    /// list scans nothing extra: AWS has no cheap "all regions" call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub regions: Option<Vec<String>>,
}

/// Narrowing the Google Cloud scan.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CloudGcloudContent {
    /// The project ids to scan. An empty list scans every project `gcloud projects list`
    /// reports ("all"), which is slow on a large organisation; naming projects skips that call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub projects: Option<Vec<String>>,
}

/// Narrowing the Azure scan.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CloudAzContent {
    /// The subscriptions to scan, each an id or a display name. An empty list scans every
    /// enabled subscription `az account list` reports ("all").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subscriptions: Option<Vec<String>>,
}

/// What one settings layer says about cloud discovery. Holds paths and names, never credentials.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CloudSettingsContent {
    /// Which providers discovery runs. Turning one off hides its Cloud card at once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub providers: Option<CloudProvidersContent>,
    /// The `aws` binary. Empty looks it up (login shell `PATH`, app `PATH`, the usual install
    /// directories).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aws_path: Option<String>,
    /// The `gcloud` binary. Empty looks it up.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gcloud_path: Option<String>,
    /// The `az` binary. Empty looks it up.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub az_path: Option<String>,
    /// The `gke-gcloud-auth-plugin` binary GKE contexts run. Empty looks it up.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gke_gcloud_auth_plugin_path: Option<String>,
    /// The `kubelogin` binary AKS contexts run. Empty looks it up.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kubelogin_path: Option<String>,
    /// What to scan on AWS.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aws: Option<CloudAwsContent>,
    /// What to scan on Google Cloud.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gcloud: Option<CloudGcloudContent>,
    /// What to scan on Azure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub az: Option<CloudAzContent>,
    /// How long a listing stays fresh, in seconds. Default 900; at least 30 and at most 86400.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_ttl_secs: Option<u64>,
    /// How many scopes (and CLI processes) run at once. Default 4; clamped to 1 to 16.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallelism: Option<u32>,
    /// How long one CLI command may run, in seconds. Default 30; clamped to 5 to 600.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_timeout_secs: Option<u64>,
}
