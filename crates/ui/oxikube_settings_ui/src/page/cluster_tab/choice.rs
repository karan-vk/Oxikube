//! [`ClusterChoice`]: one row of the cluster picker.

use gpui::SharedString;
use oxikube_app::CatalogEntry;
use oxikube_domain::ids::ClusterId;
use oxikube_settings::SettingsStore;
use serde_json::Value;

/// The key of the per-cluster blocks in `settings.json`.
const CLUSTERS_KEY: &str = "clusters";

/// A cluster the tab can edit: a catalog context, or a `clusters.<id>` block whose cluster the
/// catalog no longer lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClusterChoice {
    /// The cluster's id: the key of its block.
    pub id: ClusterId,
    /// The kubeconfig context name, when the catalog lists the cluster.
    pub context: Option<SharedString>,
    /// The kubeconfig source's label, when the catalog lists the cluster.
    pub source: Option<SharedString>,
}

impl ClusterChoice {
    /// The row of a catalog `entry`.
    pub fn from_entry(entry: &CatalogEntry) -> Self {
        Self {
            id: entry.id().clone(),
            context: Some(SharedString::from(entry.name().to_owned())),
            source: Some(SharedString::from(entry.source_label().to_owned())),
        }
    }

    /// The row of a block the catalog does not list.
    pub fn orphan(id: ClusterId) -> Self {
        Self {
            id,
            context: None,
            source: None,
        }
    }

    /// What the picker calls the cluster: its `display_name` override, else the context name,
    /// else the id.
    pub fn label(&self, store: Option<&SettingsStore>) -> SharedString {
        let named = store
            .and_then(|store| store.layer_values("display_name", Some(&self.id)).cluster)
            .and_then(|value| value.as_str().map(str::to_owned))
            .filter(|name| !name.trim().is_empty());
        match (named, &self.context) {
            (Some(name), _) => name.into(),
            (None, Some(context)) => context.clone(),
            (None, None) => SharedString::from(self.id.to_string()),
        }
    }

    /// The second line: the context name when a display name hides it, and the id.
    pub fn detail(&self, store: Option<&SettingsStore>) -> SharedString {
        let label = self.label(store);
        match &self.context {
            Some(context) if *context != label => format!("{context} \u{b7} {}", self.id),
            Some(_) => self.id.to_string(),
            None => format!("not in your kubeconfig \u{b7} {}", self.id),
        }
        .into()
    }

    /// How many keys the cluster's block holds in the user's file.
    pub fn override_count(&self, store: Option<&SettingsStore>) -> usize {
        store
            .and_then(|store| store.raw_user_settings().get(CLUSTERS_KEY))
            .and_then(|clusters| clusters.get(self.id.as_str()))
            .and_then(Value::as_object)
            .map_or(0, serde_json::Map::len)
    }
}

/// The picker's rows: the catalog's entries in the order given, then the blocks of `store`'s
/// file that no entry covers, by id.
pub fn choices(entries: &[CatalogEntry], store: Option<&SettingsStore>) -> Vec<ClusterChoice> {
    let mut rows: Vec<ClusterChoice> = entries.iter().map(ClusterChoice::from_entry).collect();
    let in_file = store
        .and_then(|store| store.raw_user_settings().get(CLUSTERS_KEY))
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|clusters| clusters.keys());
    let mut orphans: Vec<ClusterId> = in_file
        .filter_map(|key| key.parse::<ClusterId>().ok())
        .filter(|id| !rows.iter().any(|row| row.id == *id))
        .collect();
    orphans.sort();
    rows.extend(orphans.into_iter().map(ClusterChoice::orphan));
    rows
}
