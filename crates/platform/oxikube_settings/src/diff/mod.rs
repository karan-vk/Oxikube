//! The `diff` setting (E20-S09): what the cross-cluster resource diff leaves out.
//!
//! ```json
//! "diff": {
//!   "ignore_paths": [
//!     "/metadata/annotations/kubectl.kubernetes.io~1*",
//!     "/metadata/annotations/deployment.kubernetes.io~1revision"
//!   ]
//! }
//! ```
//!
//! * `ignore_paths` are JSON-pointer-like paths (RFC 6901, `~1` for a `/` inside a key) where a
//!   `*` inside a segment matches any characters, so `/metadata/annotations/team.example.com~1*`
//!   ignores a family of annotations and `/spec/template/spec/containers/*/resources` the
//!   resources of every container. The fields the server owns (`uid`, `resourceVersion`,
//!   `managedFields`, ...) are always left out and `status` can be switched on in the diff view;
//!   this list is what the user adds. A path that does not start with `/` is dropped with a
//!   warning. The list replaces the default when set.
//!
//! The setting is global. A change applies to the next diff: the binary observes
//! [`DiffSettings`] and hands the list to `oxikube_app::diff::IgnoreList`, no restart needed.

use gpui::App;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::settings::Settings;
use crate::store::SettingsStore;

#[cfg(test)]
mod tests;

/// What one settings layer says about the resource diff.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DiffSettingsContent {
    /// Paths the diff leaves out, as JSON pointers with `*` wildcards inside a segment
    /// (`/metadata/annotations/kubectl.kubernetes.io~1*`). Replaces the default list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ignore_paths: Option<Vec<String>>,
}

/// The resolved `diff` setting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffSettings {
    /// The paths the diff leaves out; only well-formed pointers.
    pub ignore_paths: Vec<String>,
}

impl DiffSettings {
    /// The default list: the annotations the tooling writes on every apply and rollout.
    pub fn default_ignore_paths() -> Vec<String> {
        [
            "/metadata/annotations/kubectl.kubernetes.io~1*",
            "/metadata/annotations/deployment.kubernetes.io~1revision",
        ]
        .map(str::to_owned)
        .to_vec()
    }
}

impl Default for DiffSettings {
    fn default() -> Self {
        Self {
            ignore_paths: Self::default_ignore_paths(),
        }
    }
}

impl Settings for DiffSettings {
    const KEY: Option<&'static str> = Some("diff");
    type Content = DiffSettingsContent;

    fn from_content(content: DiffSettingsContent) -> Self {
        let Some(paths) = content.ignore_paths else {
            return Self::default();
        };
        let (kept, dropped): (Vec<String>, Vec<String>) = paths
            .into_iter()
            .map(|path| path.trim().to_owned())
            .partition(|path| path.len() > 1 && path.starts_with('/'));
        if !dropped.is_empty() {
            tracing::warn!(
                ?dropped,
                "diff.ignore_paths entries must be JSON pointers starting with `/`: dropped"
            );
        }
        Self { ignore_paths: kept }
    }
}

crate::register_settings!(DiffSettings);

impl DiffSettings {
    /// The setting in effect, or the defaults without a settings store.
    pub fn resolve(cx: &App) -> Self {
        cx.try_global::<SettingsStore>()
            .and_then(|store| store.try_get::<Self>(None))
            .cloned()
            .unwrap_or_default()
    }
}
