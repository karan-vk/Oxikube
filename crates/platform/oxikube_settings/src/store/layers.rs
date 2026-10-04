//! The merged views of the three settings layers that every setting resolves from.
//!
//! Precedence, lowest first: `default.json` (embedded) → the user's `settings.json` →
//! `clusters.<id>` inside the user file (only for reads at that cluster's location).

use serde_json::{Map, Value};

use crate::diagnostics::{CLUSTERS_KEY, SCHEMA_KEY};
use crate::jsonc::merge_layer;

/// One cluster's override layer.
pub(crate) struct ClusterLayer {
    /// The key under `clusters` (a [`ClusterId`](oxikube_domain::ids::ClusterId) string).
    pub id: String,
    /// The raw override object from the user file.
    pub overrides: Map<String, Value>,
    /// Defaults ← user ← these overrides.
    pub merged: Value,
}

/// Defaults alone, defaults ← user, and one merge per cluster.
pub(crate) struct MergedLayers {
    /// `default.json` alone (the fallback when a first load hits a type error).
    pub defaults: Value,
    /// `default.json` with the user's root settings merged over it.
    pub root: Value,
    /// Per-cluster merges, in file order.
    pub clusters: Vec<ClusterLayer>,
}

/// A layer without the keys that are not settings (`clusters`, `$schema`).
fn settings_only(layer: &Map<String, Value>) -> Map<String, Value> {
    layer
        .iter()
        .filter(|(key, _)| key.as_str() != CLUSTERS_KEY && key.as_str() != SCHEMA_KEY)
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

impl MergedLayers {
    /// Merge `defaults` and `user`. A `clusters` entry that is not an object is skipped (the
    /// schema and unknown-key check point the user at it).
    pub fn build(defaults: &Map<String, Value>, user: &Map<String, Value>) -> Self {
        let defaults = settings_only(defaults);
        let mut root = defaults.clone();
        merge_layer(&mut root, &settings_only(user));

        let clusters = match user.get(CLUSTERS_KEY) {
            Some(Value::Object(clusters)) => clusters
                .iter()
                .filter_map(|(id, overrides)| {
                    let overrides = settings_only(overrides.as_object()?);
                    let mut merged = root.clone();
                    merge_layer(&mut merged, &overrides);
                    Some(ClusterLayer {
                        id: id.clone(),
                        overrides,
                        merged: Value::Object(merged),
                    })
                })
                .collect(),
            _ => Vec::new(),
        };

        Self {
            defaults: Value::Object(defaults),
            root: Value::Object(root),
            clusters,
        }
    }
}
