//! The `cloud` setting (E18-S07): which cloud providers discovery runs, where their CLIs are,
//! what to scan, how long listings stay fresh and how many commands run at once.
//!
//! ```json
//! "cloud": {
//!   "providers": { "aws": true, "gcloud": true, "az": true },
//!   "aws_path": "", "gcloud_path": "", "az_path": "",
//!   "gke_gcloud_auth_plugin_path": "", "kubelogin_path": "",
//!   "aws": { "profiles": [], "regions": [] },
//!   "gcloud": { "projects": [] },
//!   "az": { "subscriptions": [] },
//!   "cache_ttl_secs": 900, "parallelism": 4, "command_timeout_secs": 30
//! }
//! ```
//!
//! * `*_path` override where a CLI is; empty looks it up. The keys are the ones the adapter names
//!   in its "not found" error.
//! * Empty lists mean **all** for `aws.profiles`, `gcloud.projects` and `az.subscriptions`, and
//!   **nothing extra** for `aws.regions` (each profile's own default region is always scanned).
//!   Entries are trimmed; blanks and repeats are dropped.
//! * `cache_ttl_secs` is clamped to [`MIN_TTL_SECS`]..=[`MAX_TTL_SECS`], `parallelism` to
//!   `1..=16`, `command_timeout_secs` to `5..=600`. An out-of-range value is clamped with a
//!   warning, never an error.
//! * `providers` rejects unknown ids (a typo is reported and the previous value stays).
//!
//! The setting is global and holds paths and names only, never a credential. The binary observes
//! [`CloudSettings`] and pushes it into the discovery service and the CLI adapters; the next run
//! uses it and a provider switched off disappears at once, no restart.

mod content;

use std::path::PathBuf;
use std::time::Duration;

use gpui::App;
use oxikube_domain::cloud::CloudProvider;

use crate::settings::Settings;
use crate::store::SettingsStore;

pub use content::{
    CloudAwsContent, CloudAzContent, CloudGcloudContent, CloudProvidersContent,
    CloudSettingsContent,
};

#[cfg(test)]
mod tests;

/// The shortest cache lifetime, in seconds.
pub const MIN_TTL_SECS: u64 = 30;
/// The longest cache lifetime, in seconds (a day).
pub const MAX_TTL_SECS: u64 = 86_400;
/// The most scopes and processes allowed at once.
pub const MAX_PARALLELISM: u32 = 16;
/// The shortest command timeout, in seconds.
pub const MIN_TIMEOUT_SECS: u64 = 5;
/// The longest command timeout, in seconds.
pub const MAX_TIMEOUT_SECS: u64 = 600;

/// The resolved `cloud` setting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CloudSettings {
    /// `providers.aws`.
    pub aws_enabled: bool,
    /// `providers.gcloud`.
    pub gcloud_enabled: bool,
    /// `providers.az`.
    pub az_enabled: bool,
    /// `aws_path`; `None` looks `aws` up.
    pub aws_path: Option<PathBuf>,
    /// `gcloud_path`.
    pub gcloud_path: Option<PathBuf>,
    /// `az_path`.
    pub az_path: Option<PathBuf>,
    /// `gke_gcloud_auth_plugin_path`.
    pub gke_gcloud_auth_plugin_path: Option<PathBuf>,
    /// `kubelogin_path`.
    pub kubelogin_path: Option<PathBuf>,
    /// `aws.profiles`; empty scans every profile.
    pub aws_profiles: Vec<String>,
    /// `aws.regions`; empty adds none to each profile's default region.
    pub aws_regions: Vec<String>,
    /// `gcloud.projects`; empty scans every project.
    pub gcloud_projects: Vec<String>,
    /// `az.subscriptions` (ids or names); empty scans every enabled subscription.
    pub az_subscriptions: Vec<String>,
    /// How long a listing stays fresh.
    pub cache_ttl: Duration,
    /// How many scopes and processes run at once (1 to 16).
    pub parallelism: usize,
    /// How long one CLI command may run.
    pub command_timeout: Duration,
}

impl CloudSettings {
    /// Whether discovery runs for `provider`.
    #[must_use]
    pub fn is_enabled(&self, provider: CloudProvider) -> bool {
        match provider {
            CloudProvider::Aws => self.aws_enabled,
            CloudProvider::Gcp => self.gcloud_enabled,
            CloudProvider::Azure => self.az_enabled,
        }
    }
}

impl Default for CloudSettings {
    fn default() -> Self {
        Self {
            aws_enabled: true,
            gcloud_enabled: true,
            az_enabled: true,
            aws_path: None,
            gcloud_path: None,
            az_path: None,
            gke_gcloud_auth_plugin_path: None,
            kubelogin_path: None,
            aws_profiles: Vec::new(),
            aws_regions: Vec::new(),
            gcloud_projects: Vec::new(),
            az_subscriptions: Vec::new(),
            cache_ttl: Duration::from_secs(900),
            parallelism: 4,
            command_timeout: Duration::from_secs(30),
        }
    }
}

impl Settings for CloudSettings {
    const KEY: Option<&'static str> = Some("cloud");
    type Content = CloudSettingsContent;

    fn from_content(content: CloudSettingsContent) -> Self {
        let defaults = Self::default();
        let providers = content.providers.unwrap_or_default();
        let aws = content.aws.unwrap_or_default();
        Self {
            aws_enabled: providers.aws.unwrap_or(defaults.aws_enabled),
            gcloud_enabled: providers.gcloud.unwrap_or(defaults.gcloud_enabled),
            az_enabled: providers.az.unwrap_or(defaults.az_enabled),
            aws_path: path(content.aws_path),
            gcloud_path: path(content.gcloud_path),
            az_path: path(content.az_path),
            gke_gcloud_auth_plugin_path: path(content.gke_gcloud_auth_plugin_path),
            kubelogin_path: path(content.kubelogin_path),
            aws_profiles: names(aws.profiles),
            aws_regions: names(aws.regions),
            gcloud_projects: names(content.gcloud.and_then(|g| g.projects)),
            az_subscriptions: names(content.az.and_then(|a| a.subscriptions)),
            cache_ttl: Duration::from_secs(clamp(
                "cloud.cache_ttl_secs",
                content.cache_ttl_secs,
                defaults.cache_ttl.as_secs(),
                MIN_TTL_SECS,
                MAX_TTL_SECS,
            )),
            parallelism: usize::try_from(clamp(
                "cloud.parallelism",
                content.parallelism.map(u64::from),
                u64::try_from(defaults.parallelism).unwrap_or(4),
                1,
                u64::from(MAX_PARALLELISM),
            ))
            .unwrap_or(4),
            command_timeout: Duration::from_secs(clamp(
                "cloud.command_timeout_secs",
                content.command_timeout_secs,
                defaults.command_timeout.as_secs(),
                MIN_TIMEOUT_SECS,
                MAX_TIMEOUT_SECS,
            )),
        }
    }
}

crate::register_settings!(CloudSettings);

impl CloudSettings {
    /// The setting in effect, or the defaults without a settings store.
    pub fn resolve(cx: &App) -> Self {
        cx.try_global::<SettingsStore>()
            .and_then(|store| store.try_get::<Self>(None))
            .cloned()
            .unwrap_or_default()
    }
}

/// A trimmed, non-empty path, or `None` (look it up).
fn path(value: Option<String>) -> Option<PathBuf> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
}

/// `values` trimmed, without blanks or repeats, in order.
fn names(values: Option<Vec<String>>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for value in values.unwrap_or_default() {
        let value = value.trim();
        if !value.is_empty() && !out.iter().any(|seen| seen == value) {
            out.push(value.to_owned());
        }
    }
    out
}

/// `value` (or `default`) kept within `min..=max`, with a warning when it was moved.
fn clamp(key: &str, value: Option<u64>, default: u64, min: u64, max: u64) -> u64 {
    let Some(value) = value else { return default };
    let clamped = value.clamp(min, max);
    if clamped != value {
        tracing::warn!(
            key,
            value,
            min,
            max,
            "cloud setting out of range: clamped to {clamped}"
        );
    }
    clamped
}
