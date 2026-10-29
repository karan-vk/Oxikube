//! The `kustomize` setting (E20-S04): where the `kustomize` binary is.
//!
//! ```json
//! "kustomize": { "path": "" }
//! ```
//!
//! `path` is the binary that builds overlays. Empty (the default) autodetects: a standalone
//! `kustomize` on the login shell's `PATH` or in the usual install directories, then
//! `kubectl kustomize`. A file named `kubectl` selects `kubectl kustomize`; any other name is
//! taken as a standalone kustomize. A path that is not an executable file means "not
//! installed" (the kustomize actions are hidden); it is never replaced by a `PATH` lookup.
//!
//! The setting is global (the tools belong to the machine). Changing `kustomize.path` takes
//! effect without a restart: the binary observes [`KustomizeSettings`] and detects again.

use std::path::PathBuf;

use gpui::App;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::settings::Settings;
use crate::store::SettingsStore;

#[cfg(test)]
mod tests;

/// What one settings layer says about kustomize.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct KustomizeSettingsContent {
    /// The `kustomize` (or `kubectl`) binary. Empty autodetects.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// The resolved `kustomize` setting.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KustomizeSettings {
    /// The configured binary; `None` autodetects.
    pub path: Option<PathBuf>,
}

impl Settings for KustomizeSettings {
    const KEY: Option<&'static str> = Some("kustomize");
    type Content = KustomizeSettingsContent;

    fn from_content(content: KustomizeSettingsContent) -> Self {
        Self {
            path: content
                .path
                .as_deref()
                .map(str::trim)
                .filter(|path| !path.is_empty())
                .map(PathBuf::from),
        }
    }
}

crate::register_settings!(KustomizeSettings);

impl KustomizeSettings {
    /// The setting in effect, or the defaults without a settings store.
    pub fn resolve(cx: &App) -> Self {
        cx.try_global::<SettingsStore>()
            .and_then(|store| store.try_get::<Self>(None))
            .cloned()
            .unwrap_or_default()
    }
}
