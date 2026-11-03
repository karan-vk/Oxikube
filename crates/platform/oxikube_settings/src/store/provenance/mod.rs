//! Where a setting's effective value comes from (E21-S04): the layer badge of the settings page,
//! the reset button's visibility (E21-S05) and the precedence view of the cluster tab (E21-S06).
//!
//! A field is named by its dotted path in `settings.json` (`terminal.font_size`, `read_only`).
//! The answer reads the layers the store already keeps; nothing is parsed or merged per call.

#[cfg(test)]
mod tests;

use oxikube_domain::ids::ClusterId;
use serde_json::{Map, Value};

use super::SettingsStore;
use super::layers::ClusterLayer;
use crate::diagnostics::CLUSTERS_KEY;

/// The settings layer a field's effective value comes from, lowest first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SettingSource {
    /// The embedded `default.json` (or no value at all: the type's own fallback).
    Default,
    /// The user's `settings.json`, outside any `clusters` block.
    User,
    /// The `clusters.<id>` block of the cluster asked about.
    Cluster,
}

impl SettingSource {
    /// A short, human name for the layer: `Default`, `User`, `Cluster`.
    pub fn label(self) -> &'static str {
        match self {
            Self::Default => "Default",
            Self::User => "User",
            Self::Cluster => "Cluster",
        }
    }
}

/// The value at the dotted `path` inside `root`, if every segment is present.
fn lookup<'a>(root: &'a Map<String, Value>, path: &str) -> Option<&'a Value> {
    let mut segments = path.split('.');
    let mut current = root.get(segments.next()?)?;
    for segment in segments {
        current = current.as_object()?.get(segment)?;
    }
    Some(current)
}

impl SettingsStore {
    /// The layer the effective value of the field at the dotted `path` comes from, read at
    /// `cluster` (`None`: the global value). The highest layer that sets the field wins: the
    /// cluster's own block, then the user's file, else the defaults. A path that names an object
    /// counts as set by a layer that sets any part of it.
    pub fn field_source(&self, path: &str, cluster: Option<&ClusterId>) -> SettingSource {
        if path.is_empty() || path.split('.').next() == Some(CLUSTERS_KEY) {
            return SettingSource::Default;
        }
        if self
            .cluster_layer(cluster)
            .and_then(|layer| lookup(&layer.overrides, path))
            .is_some()
        {
            return SettingSource::Cluster;
        }
        if lookup(&self.user, path).is_some() {
            return SettingSource::User;
        }
        SettingSource::Default
    }

    /// The effective value of the field at the dotted `path`, read at `cluster` (`None`: the
    /// global value): the merge of every layer. `None` when no layer sets it.
    pub fn effective_value(&self, path: &str, cluster: Option<&ClusterId>) -> Option<&Value> {
        let merged = self
            .cluster_layer(cluster)
            .map_or(&self.layers.root, |layer| &layer.merged);
        lookup(merged.as_object()?, path)
    }

    /// The value `default.json` gives the field at the dotted `path`, if any.
    pub fn default_value(&self, path: &str) -> Option<&Value> {
        lookup(self.layers.defaults.as_object()?, path)
    }

    /// The layer of `cluster`, if the user's file has a block for it.
    fn cluster_layer(&self, cluster: Option<&ClusterId>) -> Option<&ClusterLayer> {
        let cluster = cluster?;
        self.layers
            .clusters
            .iter()
            .find(|layer| layer.id == cluster.as_str())
    }
}
