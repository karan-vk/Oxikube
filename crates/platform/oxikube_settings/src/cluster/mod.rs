//! The per-cluster settings layer (E06-S08): `clusters.<id>` overrides for display name, colour,
//! read-only, default namespace, terminal working directory, the node shell pod template (image,
//! pull secret, and the `node_shell` block: E09-S09), Prometheus location, accessible namespaces, the exec plugin policy
//! and the watch budget (the `watch_budget` block: E04-F543).
//!
//! The layer mechanics (default.json, then the user's top-level values, then `clusters.<id>`,
//! merged field by field, a type error keeping the previous value, unknown keys reported) belong
//! to [`SettingsStore`](crate::SettingsStore) and apply to every setting. This module is the
//! first setting that is *about* clusters:
//!
//! - [`ClusterSettingsContent`] is the file shape: root-level keys, so each is valid at the top
//!   of the file (a default for all clusters) and under `clusters.<id>` (an override);
//! - [`ClusterSettings`] is the resolved value, wrapping the plain
//!   [`ClusterPrefs`](oxikube_ports::ClusterPrefs) the app layer understands, with
//!   [`ClusterSettings::resolve`], [`ClusterSettings::observe_cluster`],
//!   [`ClusterSettings::table`] and the comment-preserving writers in `edit`;
//! - [`fields`] has the validated field types: a URL without credentials and a keychain entry
//!   name, so the file never holds a secret.
//!
//! Lookups are O(1): the store keeps a hash map from cluster id to resolved value for the
//! clusters that have overrides and falls back to the global value for the rest.

mod content;
mod edit;
pub mod fields;
mod node_shell;
mod resolved;
#[cfg(test)]
mod tests;
mod watch_budget;

pub use content::{ClusterSettingsContent, PrometheusContent};
pub use node_shell::{
    ImagePullPolicy, NodeShellContent, TaintEffect, TolerationContent, TolerationOperator,
};
pub use resolved::ClusterSettings;
pub use watch_budget::WatchBudgetContent;
