//! [`NamespacePrefs`]: what is remembered per cluster about namespaces, and where.

use oxikube_domain::OxiResult;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::session::{NamespaceFavourites, NamespaceSelection};
use oxikube_ports::{StateKey, StatePort, StatePortExt};
use serde::{Deserialize, Serialize};

/// The namespace state remembered for one cluster in `StatePort` (SQLite, never settings).
///
/// Stored as one kv value under [`prefs_key`]. It holds names only, never credentials.
/// Every field defaults, so a record written by an older build still loads.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NamespacePrefs {
    /// The selection the session restores on connect.
    pub selection: NamespaceSelection,
    /// The pinned namespaces; `1`-`9` jump to the first nine (see
    /// [`slot_selection`](super::slot_selection)).
    pub favourites: NamespaceFavourites,
    /// Names the user typed because the cluster refuses to list namespaces (RBAC). Offered
    /// as the namespace list while listing is forbidden. E06-S08's per-cluster
    /// "accessible namespaces" setting feeds the same list.
    pub typed: Vec<String>,
}

impl NamespacePrefs {
    /// Adds `name` to the typed names. Returns whether it was new. Blank names are ignored.
    pub fn add_typed(&mut self, name: &str) -> bool {
        let name = name.trim();
        if name.is_empty() || self.typed.iter().any(|n| n == name) {
            return false;
        }
        self.typed.push(name.to_owned());
        self.typed.sort();
        true
    }

    /// Removes `name` from the typed names. Returns whether it was there.
    pub fn remove_typed(&mut self, name: &str) -> bool {
        let before = self.typed.len();
        self.typed.retain(|n| n != name.trim());
        self.typed.len() != before
    }
}

/// The kv key holding `cluster`'s [`NamespacePrefs`]: `cluster/<id>/namespaces`.
pub fn prefs_key(cluster: &ClusterId) -> StateKey {
    // A `ClusterId` is 16 hex characters, so the key is always valid.
    StateKey::new(format!("cluster/{cluster}/namespaces")).expect("cluster ids make valid keys")
}

/// Reads `cluster`'s prefs. A missing record is the default; a record that no longer parses
/// is logged and treated as missing (a stale format must not lock the user out of the
/// selector). Storage errors propagate.
pub(super) async fn read(state: &dyn StatePort, cluster: &ClusterId) -> OxiResult<NamespacePrefs> {
    match state.kv_get_as::<NamespacePrefs>(&prefs_key(cluster)).await {
        Ok(found) => Ok(found.unwrap_or_default()),
        Err(err) if err.kind() == oxikube_domain::ErrorKind::Validation => {
            tracing::warn!(%cluster, %err, "ignoring unreadable namespace prefs");
            Ok(NamespacePrefs::default())
        }
        Err(err) => Err(err),
    }
}

/// Writes `cluster`'s prefs.
pub(super) async fn write(
    state: &dyn StatePort,
    cluster: &ClusterId,
    prefs: &NamespacePrefs,
) -> OxiResult<()> {
    state.kv_set_as(&prefs_key(cluster), prefs).await
}
