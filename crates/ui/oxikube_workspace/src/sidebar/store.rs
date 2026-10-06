//! [`SidebarStore`]: which sidebar groups the user opened or closed, per cluster, in the
//! [`StatePort`].
//!
//! One row per cluster in the state table [`SIDEBAR_TABLE`], keyed by the cluster id:
//! `{ "version": 1, "open": { "<row id>": true|false } }`. Only the user's explicit choices are
//! stored; a group nobody touched follows its default (sections open, custom-resource groups
//! closed). Section and group ids and booleans only: nothing secret (non-negotiable 5).

use std::collections::BTreeMap;
use std::sync::Arc;

use oxikube_domain::OxiResult;
use oxikube_domain::ids::ClusterId;
use oxikube_ports::{StateKey, StatePort, StatePortExt as _, StateTable};
use serde::{Deserialize, Serialize};

/// The state table the sidebar state is stored in, one row per cluster.
pub const SIDEBAR_TABLE: &str = "cluster_sidebar";

/// The version this build writes.
pub const SIDEBAR_VERSION: u32 = 1;

/// What is saved for one cluster's sidebar.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedSidebar {
    /// [`SIDEBAR_VERSION`] of the build that wrote it.
    #[serde(default)]
    pub version: u32,
    /// The groups the user opened (`true`) or closed (`false`), by row id.
    #[serde(default)]
    pub open: BTreeMap<String, bool>,
}

/// Reads and writes one cluster's [`SavedSidebar`]. Every call is async: the port runs it off the
/// UI thread.
#[derive(Clone)]
pub struct SidebarStore {
    state: Arc<dyn StatePort>,
    table: StateTable,
    key: StateKey,
}

impl SidebarStore {
    /// A store for `cluster`.
    ///
    /// # Errors
    ///
    /// `Validation` when the cluster id is not a valid state key.
    pub fn new(state: Arc<dyn StatePort>, cluster: &ClusterId) -> OxiResult<Self> {
        Ok(Self {
            state,
            table: StateTable::new(SIDEBAR_TABLE)?,
            key: StateKey::new(cluster.to_string())?,
        })
    }

    /// The saved state, `None` when nothing was saved. A row of an unexpected shape or a newer
    /// version reads as `None` too (and is replaced by the next save): a bad row must never break
    /// the sidebar.
    ///
    /// # Errors
    ///
    /// The port's errors (database I/O).
    pub async fn load(&self) -> OxiResult<Option<SavedSidebar>> {
        match self
            .state
            .table_get_as::<SavedSidebar>(&self.table, &self.key)
            .await
        {
            Ok(Some(saved)) if saved.version <= SIDEBAR_VERSION => Ok(Some(saved)),
            Ok(Some(saved)) => {
                tracing::warn!(
                    version = saved.version,
                    "saved sidebar state is from a newer build: ignored"
                );
                Ok(None)
            }
            Ok(None) => Ok(None),
            Err(error) if error.kind() == oxikube_domain::ErrorKind::Validation => {
                tracing::warn!(%error, "saved sidebar state is unreadable: ignored");
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    /// Writes the user's open and closed groups, replacing the previous row.
    ///
    /// # Errors
    ///
    /// The port's errors (database I/O).
    pub async fn save(&self, open: &BTreeMap<String, bool>) -> OxiResult<()> {
        let row = SavedSidebar {
            version: SIDEBAR_VERSION,
            open: open.clone(),
        };
        self.state.table_put_as(&self.table, &self.key, &row).await
    }
}

impl std::fmt::Debug for SidebarStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SidebarStore")
            .field("cluster", &self.key.as_str())
            .finish_non_exhaustive()
    }
}
