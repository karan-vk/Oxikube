//! [`ClusterChoice`]: one row of the cluster picker.

use gpui::SharedString;
use oxikube_app::CatalogEntry;
use oxikube_domain::ids::ClusterId;
use oxikube_settings::SettingsStore;
use serde_json::Value;

/// The key of the per-cluster blocks in `settings.json`.
const CLUSTERS_KEY: &str = "clusters";

/// What a row of the picker shows, worked out when the list or the file changes (never per
/// frame: each answer reads the store).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClusterRowText {
    /// The first line ([`ClusterChoice::label`]).
    pub label: SharedString,
    /// The second line ([`ClusterChoice::detail`]).
    pub detail: SharedString,
    /// How many keys the cluster's block holds ([`ClusterChoice::override_count`]).
    pub overrides: usize,
}

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
        // The block itself, looked up by key: the store's own per-cluster search is linear in
        // the clusters, and the picker asks once per row.
        let named = self
            .block(store)
            .and_then(|block| block.get("display_name"))
            .and_then(Value::as_str)
            .filter(|name| !name.trim().is_empty());
        match (named, &self.context) {
            (Some(name), _) => SharedString::from(name.to_owned()),
            (None, Some(context)) => context.clone(),
            (None, None) => SharedString::from(self.id.to_string()),
        }
    }

    /// The second line: the context name when a display name hides it, and the id.
    pub fn detail(&self, store: Option<&SettingsStore>) -> SharedString {
        self.detail_under(&self.label(store))
    }

    /// Everything the row shows, read from `store` once.
    pub fn row_text(&self, store: Option<&SettingsStore>) -> ClusterRowText {
        let label = self.label(store);
        ClusterRowText {
            detail: self.detail_under(&label),
            overrides: self.override_count(store),
            label,
        }
    }

    fn detail_under(&self, label: &SharedString) -> SharedString {
        match &self.context {
            Some(context) if context != label => format!("{context} \u{b7} {}", self.id),
            Some(_) => self.id.to_string(),
            None => format!("not in your kubeconfig \u{b7} {}", self.id),
        }
        .into()
    }

    /// How many keys the cluster's block holds in the user's file.
    pub fn override_count(&self, store: Option<&SettingsStore>) -> usize {
        self.block(store).map_or(0, serde_json::Map::len)
    }

    /// The cluster's `clusters.<id>` block in the user's file.
    fn block<'a>(
        &self,
        store: Option<&'a SettingsStore>,
    ) -> Option<&'a serde_json::Map<String, Value>> {
        store?
            .raw_user_settings()
            .get(CLUSTERS_KEY)?
            .get(self.id.as_str())?
            .as_object()
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
