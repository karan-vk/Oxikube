//! The `helm` setting (E16-S11): where the `helm` binary is, whether installs and upgrades use
//! server-side apply, and the chart repositories offered by default.
//!
//! ```json
//! "helm": {
//!   "path": "",
//!   "server_side_apply": false,
//!   "default_repos": [{ "name": "bitnami", "url": "https://charts.bitnami.com/bitnami" }]
//! }
//! ```
//!
//! * `path` is the `helm` binary. Empty (the default) autodetects: `PATH`, then the usual
//!   install directories. A path that does not exist is not an error here: detection reports
//!   `BinaryMissing` and the Helm views show guidance with a "Locate helm" action.
//! * `server_side_apply` (default `false`) asks installs and upgrades to pass `--server-side`.
//!   Helm supports it from 3.13; [`server_side_warning`] says what to tell the user when the
//!   detected binary is older.
//! * `default_repos` is the list of `{ name, url }` the Repositories section offers to add.
//!   An entry whose URL is not `http(s)`, carries credentials (`user:pass@`; a setting must not
//!   hold a secret) or has an empty or duplicate name is dropped with a warning.
//!
//! The setting is global (Helm repositories and the binary belong to the machine, not to a
//! cluster). Changing `helm.path` takes effect without a restart: the binary observes
//! [`HelmSettings`] and re-runs detection (`oxikube_helm::HelmCli::reconfigure`).

use std::collections::HashSet;
use std::path::PathBuf;

use gpui::App;
use oxikube_domain::helm::{HelmCapabilities, HelmVersion, RepoEntry};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::settings::Settings;
use crate::store::SettingsStore;

#[cfg(test)]
mod tests;

/// One `{ name, url }` of `helm.default_repos` as written in `settings.json`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HelmRepoContent {
    /// The repository alias (`bitnami`): letters, digits, `-`, `_` and `.`.
    #[serde(default)]
    pub name: String,
    /// The repository index URL (`https://charts.bitnami.com/bitnami`). `http(s)` only, and
    /// without credentials.
    #[serde(default)]
    pub url: String,
}

/// What one settings layer says about Helm.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HelmSettingsContent {
    /// The `helm` binary. Empty autodetects (`PATH`, then the usual install directories).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Pass `--server-side` to install and upgrade (Helm 3.13 or newer). Default `false`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_side_apply: Option<bool>,
    /// The chart repositories the Repositories section offers to add. Default: bitnami,
    /// ingress-nginx and prometheus-community.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_repos: Option<Vec<HelmRepoContent>>,
}

/// The resolved `helm` setting.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HelmSettings {
    /// The configured binary; `None` autodetects.
    pub path: Option<PathBuf>,
    /// Whether install and upgrade pass `--server-side`.
    pub server_side_apply: bool,
    /// The repositories offered by default (validated).
    pub default_repos: Vec<RepoEntry>,
}

impl Settings for HelmSettings {
    const KEY: Option<&'static str> = Some("helm");
    type Content = HelmSettingsContent;

    fn from_content(content: HelmSettingsContent) -> Self {
        Self {
            path: content
                .path
                .as_deref()
                .map(str::trim)
                .filter(|path| !path.is_empty())
                .map(PathBuf::from),
            server_side_apply: content.server_side_apply.unwrap_or(false),
            default_repos: valid_repos(content.default_repos.unwrap_or_default()),
        }
    }
}

crate::register_settings!(HelmSettings);

impl HelmSettings {
    /// The setting in effect, or the defaults without a settings store.
    pub fn resolve(cx: &App) -> Self {
        cx.try_global::<SettingsStore>()
            .and_then(|store| store.try_get::<Self>(None))
            .cloned()
            .unwrap_or_default()
    }
}

/// Keeps the repositories that are usable, in order, and warns about the others.
fn valid_repos(repos: Vec<HelmRepoContent>) -> Vec<RepoEntry> {
    let mut seen = HashSet::new();
    let mut valid = Vec::with_capacity(repos.len());
    for repo in repos {
        let name = repo.name.trim();
        let url = repo.url.trim();
        match problem(name, url) {
            Some(why) => {
                // The URL is not logged: a mistaken entry may carry a credential.
                tracing::warn!(name, "helm.default_repos entry ignored: {why}");
            }
            None if !seen.insert(name.to_owned()) => {
                tracing::warn!(name, "helm.default_repos entry ignored: duplicate name");
            }
            None => valid.push(RepoEntry {
                name: name.to_owned(),
                url: url.to_owned(),
            }),
        }
    }
    valid
}

/// Why `name`/`url` cannot be a repository, if they cannot.
fn problem(name: &str, url: &str) -> Option<&'static str> {
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_alphanumeric() || "-_.".contains(c))
    {
        return Some("the name must be letters, digits, '-', '_' or '.'");
    }
    let Some((scheme, rest)) = url.split_once("://") else {
        return Some("the URL must start with http:// or https://");
    };
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return Some("the URL must start with http:// or https://");
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.is_empty() {
        return Some("the URL has no host");
    }
    if authority.contains('@') {
        return Some("the URL must not contain credentials");
    }
    None
}

/// The first Helm that takes `--server-side` on install and upgrade.
pub const SERVER_SIDE_MINIMUM: HelmVersion = HelmVersion::new(3, 13, 0);

/// What to show the user when `helm.server_side_apply` is on but the detected binary cannot do
/// it: `Some(text)` for a binary older than [`SERVER_SIDE_MINIMUM`], `None` when the flag is
/// off, no binary is known, or the binary supports it.
pub fn server_side_warning(setting: &HelmSettings, caps: &HelmCapabilities) -> Option<String> {
    let version = caps.cli_version.filter(|_| caps.cli_available)?;
    (setting.server_side_apply && version < SERVER_SIDE_MINIMUM).then(|| {
        format!(
            "helm.server_side_apply is on, but Helm {version} cannot do server-side apply \
             (needs {SERVER_SIDE_MINIMUM} or newer). Installs and upgrades use the default \
             apply until Helm is upgraded."
        )
    })
}
