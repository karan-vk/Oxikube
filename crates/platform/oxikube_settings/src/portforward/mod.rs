//! The `portforward` setting (E15-S08): where a port-forward listens by default, whether it may
//! start on a read-only cluster, which local ports automatic allocation uses, and whether the
//! app annotates the objects it forwards.
//!
//! ```json
//! "portforward": {
//!   "default_bind": "127.0.0.1",
//!   "allowed_in_readonly": false,
//!   "port_range": [20000, 40000],
//!   "annotations": false
//! }
//! ```
//!
//! * `default_bind` is the address a forward listens on unless the request names another. It must
//!   parse as an IP address (a wrong type keeps the previous value and is reported). Loopback by
//!   default: a forward exposes a cluster endpoint to whoever can reach the listener.
//! * `allowed_in_readonly` lets forwards start on a read-only cluster. A forward changes nothing
//!   in the cluster, but the default stays strict.
//! * `port_range` is `[first, last]`, inclusive, unprivileged (`first >= 1024`, `first <= last`):
//!   a forward that asks for a free port gets one from it. Unset keeps the OS-assigned default; a
//!   range that breaks the rules is ignored with a warning.
//! * `annotations` (default `false`) opts in to annotating forwarded objects.
//!
//! Objects merge across layers, so `clusters.<id>.portforward` overrides single keys for one
//! cluster. [`PortForwardSettings::table`] hands the binary the resolved values of every cluster;
//! it pushes them into the app's `PortForwardManager` on each change, which applies them to
//! forwards started afterwards and leaves running ones alone.

use std::net::IpAddr;

use gpui::App;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::portforward::{ForwardSpec, PortRange};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::settings::Settings;
use crate::store::SettingsStore;

#[cfg(test)]
mod tests;

/// What one settings layer says about port-forwards.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PortForwardSettingsContent {
    /// The IP address a forward listens on unless the request names another. Default
    /// `127.0.0.1` (loopback only). Anything else exposes the forwarded port to the network.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_bind: Option<IpAddr>,
    /// Allow forwards on a read-only cluster. Default `false`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_in_readonly: Option<bool>,
    /// `[first, last]`, the local ports automatic allocation picks from: inclusive,
    /// `first` at least 1024 and not above `last`. Unset lets the OS pick.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 2, max = 2))]
    pub port_range: Option<[u16; 2]>,
    /// Annotate forwarded objects (opt-in). Default `false`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<bool>,
}

/// The resolved `portforward` setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PortForwardSettings {
    /// Where a forward listens unless told otherwise.
    pub default_bind: IpAddr,
    /// Whether forwards may start on a read-only cluster.
    pub allowed_in_readonly: bool,
    /// The ports automatic allocation uses; `None` lets the OS pick.
    pub port_range: Option<PortRange>,
    /// Whether forwarded objects are annotated.
    pub annotations: bool,
}

impl Default for PortForwardSettings {
    fn default() -> Self {
        Self {
            default_bind: ForwardSpec::DEFAULT_BIND,
            allowed_in_readonly: false,
            port_range: None,
            annotations: false,
        }
    }
}

impl Settings for PortForwardSettings {
    const KEY: Option<&'static str> = Some("portforward");
    type Content = PortForwardSettingsContent;

    fn from_content(content: PortForwardSettingsContent) -> Self {
        let defaults = Self::default();
        Self {
            default_bind: content.default_bind.unwrap_or(defaults.default_bind),
            allowed_in_readonly: content
                .allowed_in_readonly
                .unwrap_or(defaults.allowed_in_readonly),
            port_range: content.port_range.and_then(range),
            annotations: content.annotations.unwrap_or(defaults.annotations),
        }
    }
}

fn range([first, last]: [u16; 2]) -> Option<PortRange> {
    let range = PortRange::new(first, last);
    if range.is_none() {
        tracing::warn!(
            first,
            last,
            "portforward.port_range must be [first, last] with first >= {} and first <= last: \
             ignored, the OS picks free ports",
            PortRange::MIN_START
        );
    }
    range
}

crate::register_settings!(PortForwardSettings);

impl PortForwardSettings {
    /// The settings in effect for `cluster` (its `clusters.<id>.portforward` over the user's and
    /// the defaults), or the defaults without a settings store.
    pub fn resolve(cluster: &ClusterId, cx: &App) -> Self {
        cx.try_global::<SettingsStore>()
            .and_then(|store| store.try_get::<Self>(Some(crate::SettingsLocation { cluster })))
            .copied()
            .unwrap_or_default()
    }

    /// The resolved setting: the global value and every cluster that overrides any of it. Keys
    /// of `clusters` that are not cluster ids are skipped. Without a store, the defaults.
    pub fn table(cx: &App) -> (Self, Vec<(ClusterId, Self)>) {
        let Some(store) = cx.try_global::<SettingsStore>() else {
            return (Self::default(), Vec::new());
        };
        let global = store.try_get::<Self>(None).copied().unwrap_or_default();
        let clusters = store
            .cluster_values::<Self>()
            .filter_map(|(key, settings)| Some((key.parse().ok()?, *settings)))
            .collect();
        (global, clusters)
    }
}
