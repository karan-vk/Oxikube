//! The `metrics` setting (E13-S02): how utilisation is computed.
//!
//! ```json
//! "metrics": {
//!   "gpu_vendors": ["nvidia.com/gpu", "amd.com/gpu", "gpu.intel.com/i915", "gpu.intel.com/xe"]
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
//! push a new one: edits take effect on the next computation, without a restart. Later metrics
//! stories add their keys (polling cadence, Prometheus history) to the same block.

use gpui::App;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::utilisation::{GpuVendors, GpuVendorsTable};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::settings::Settings;
use crate::store::SettingsStore;

/// What one settings layer says about metrics.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MetricsSettingsContent {
    /// Extended resource names counted as GPUs, for example `nvidia.com/gpu`. Each is summed as
    /// a whole number per pod, node and namespace. This list replaces the default as a whole;
    /// an empty list turns GPU accounting off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpu_vendors: Option<Vec<String>>,
}

/// The resolved `metrics` setting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MetricsSettings {
    /// The resource names counted as GPUs.
    pub gpu_vendors: GpuVendors,
}

impl Settings for MetricsSettings {
    const KEY: Option<&'static str> = Some("metrics");
    type Content = MetricsSettingsContent;

    fn from_content(content: MetricsSettingsContent) -> Self {
        Self {
            gpu_vendors: content
                .gpu_vendors
                .map_or_else(GpuVendors::default, GpuVendors::new),
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
}

#[cfg(test)]
mod tests;
