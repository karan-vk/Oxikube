//! The `metrics` setting (E13-S02, E13-S03): how utilisation is computed, how often live usage
//! is polled and when it turns amber or red.
//!
//! ```json
//! "metrics": {
//!   "gpu_vendors": ["nvidia.com/gpu", "amd.com/gpu", "gpu.intel.com/i915", "gpu.intel.com/xe"],
//!   "poll_interval_secs": 15,
//!   "thresholds": { "cpu": { "warn": 70, "critical": 90 }, "memory": { "warn": 70, "critical": 90 } }
//! }
//! ```
//!
//! `gpu_vendors` lists the extended resource names that count as GPUs in the requests, limits and
//! allocation figures (pods, nodes, namespaces). A GPU is a whole-number count, summed per name.
//! The list is replaced as a whole by a layer (arrays do not merge); under `clusters.<id>.metrics`
//! it overrides the list for one cluster. Blank entries and repeats are ignored.
//!
//! [`MetricsSettings::gpu_table`] hands the app layer (which has no `gpui`) the resolved lists for
//! every cluster, and [`MetricsSettings::observe`] (from [`Settings`]) tells the binary when to
//! push a new one: edits take effect on the next computation, without a restart.
//!
//! `poll_interval_secs` (5 to 600, default 15) is how often metrics-server is asked while a
//! table shows usage, and `thresholds` are k9s' `thresholds.cpu` / `thresholds.memory`: the
//! percent of a limit (or of a node's allocatable) at which a cell turns amber (`warn`) and red
//! (`critical`). Objects merge across layers, so a cluster can override one level alone.
//! [`MetricsSettings::table`] hands every cluster's resolved values to the binary, which pushes
//! them into the app's `MetricsService` on every change.

use gpui::App;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::utilisation::{GpuVendors, GpuVendorsTable};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::settings::Settings;
use crate::store::SettingsStore;

mod live;

pub use live::{
    DEFAULT_CRITICAL_PERCENT, DEFAULT_POLL_INTERVAL_SECS, DEFAULT_WARN_PERCENT,
    MAX_POLL_INTERVAL_SECS, MIN_POLL_INTERVAL_SECS, ThresholdsContent, UsageLevel,
    UsageLevelContent,
};

/// What one settings layer says about metrics.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MetricsSettingsContent {
    /// Extended resource names counted as GPUs, for example `nvidia.com/gpu`. Each is summed as
    /// a whole number per pod, node and namespace. This list replaces the default as a whole;
    /// an empty list turns GPU accounting off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpu_vendors: Option<Vec<String>>,
    /// Seconds between two polls of metrics-server while a table shows live usage (5 to 600;
    /// default 15). Polling stops while nothing shows it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poll_interval_secs: Option<u64>,
    /// When a usage percent turns amber (`warn`) or red (`critical`), as k9s' `thresholds`:
    /// the percent of a pod's or container's limit, or of a node's allocatable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thresholds: Option<ThresholdsContent>,
}

/// The resolved `metrics` setting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MetricsSettings {
    /// The resource names counted as GPUs.
    pub gpu_vendors: GpuVendors,
    /// Seconds between two polls, within 5 to 600.
    pub poll_interval_secs: u64,
    /// CPU's warn and critical levels.
    pub cpu: UsageLevel,
    /// Memory's warn and critical levels.
    pub memory: UsageLevel,
}

impl Settings for MetricsSettings {
    const KEY: Option<&'static str> = Some("metrics");
    type Content = MetricsSettingsContent;

    fn from_content(content: MetricsSettingsContent) -> Self {
        let thresholds = content.thresholds.unwrap_or_default();
        Self {
            gpu_vendors: content
                .gpu_vendors
                .map_or_else(GpuVendors::default, GpuVendors::new),
            poll_interval_secs: live::poll_interval(content.poll_interval_secs),
            cpu: UsageLevel::of(thresholds.cpu.as_ref()),
            memory: UsageLevel::of(thresholds.memory.as_ref()),
        }
    }
}

crate::register_settings!(MetricsSettings);

impl MetricsSettings {
    /// The GPU vendor list of every cluster: the global list plus each cluster that overrides
    /// it. Keys of `clusters` that are not cluster ids are skipped (the cluster layer reports
    /// them). Without a store or without this setting registered, the defaults.
    pub fn gpu_table(cx: &App) -> GpuVendorsTable {
        let Some(store) = cx.try_global::<SettingsStore>() else {
            return GpuVendorsTable::default();
        };
        let default = store
            .try_get::<MetricsSettings>(None)
            .map(|settings| settings.gpu_vendors.clone())
            .unwrap_or_default();
        let mut table = GpuVendorsTable::new(default);
        for (key, settings) in store.cluster_values::<MetricsSettings>() {
            if let Ok(id) = key.parse::<ClusterId>() {
                table = table.with_cluster(id, settings.gpu_vendors.clone());
            }
        }
        table
    }

    /// The resolved setting: the global value and every cluster that overrides any of it. Keys
    /// of `clusters` that are not cluster ids are skipped. Without a store, the defaults.
    pub fn table(cx: &App) -> (MetricsSettings, Vec<(ClusterId, MetricsSettings)>) {
        let Some(store) = cx.try_global::<SettingsStore>() else {
            return (
                Self::from_content(MetricsSettingsContent::default()),
                Vec::new(),
            );
        };
        let global = store
            .try_get::<MetricsSettings>(None)
            .cloned()
            .unwrap_or_else(|| Self::from_content(MetricsSettingsContent::default()));
        let clusters = store
            .cluster_values::<MetricsSettings>()
            .filter_map(|(key, settings)| Some((key.parse().ok()?, settings.clone())))
            .collect();
        (global, clusters)
    }
}

#[cfg(test)]
mod tests;
