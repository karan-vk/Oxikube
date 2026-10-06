//! [`ClusterTabsStore`]: which cluster tabs a window had open, in what order, in the
//! [`StatePort`].
//!
//! One row per window in the state table [`CLUSTER_TABS_TABLE`]: `{ "version": 1, "open": [ids],
//! "active": id }`. Cluster ids and nothing else: no names, no credentials (non-negotiable 5).
//! Each cluster's own pane layout is a separate row in the layout table, keyed by the cluster
//! ([`cluster_layout_key`]), so a cluster that is closed and opened again gets its layout back.
//! Reconnecting the clusters on the next launch is session restore (E06-S11), which reads
//! [`ClusterTabsStore::load`].

use std::sync::Arc;

use oxikube_domain::OxiResult;
use oxikube_domain::ids::ClusterId;
use oxikube_ports::{StateKey, StatePort, StatePortExt as _, StateTable};
use serde::{Deserialize, Serialize};

/// The state table the open cluster tabs are stored in, one row per window.
pub const CLUSTER_TABS_TABLE: &str = "cluster_tabs";

/// The version this build writes.
pub const CLUSTER_TABS_VERSION: u32 = 1;

/// The layout row of a cluster's own workspace (in the table of
/// [`LayoutStore`](crate::persistence::LayoutStore)): `cluster:<id>`.
pub fn cluster_layout_key(cluster: &ClusterId) -> String {
    format!("cluster:{cluster}")
}

/// The open cluster tabs of one window.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedTabs {
    /// [`CLUSTER_TABS_VERSION`] of the build that wrote it.
    #[serde(default)]
    pub version: u32,
    /// The clusters with an open tab, in tab order.
    #[serde(default)]
    pub open: Vec<ClusterId>,
    /// The cluster whose tab was displayed.
    #[serde(default)]
    pub active: Option<ClusterId>,
}

impl SavedTabs {
    /// The tabs `open` in order, with `active` displayed.
    pub fn new(open: Vec<ClusterId>, active: Option<ClusterId>) -> Self {
        Self {
            version: CLUSTER_TABS_VERSION,
            open,
            active,
        }
    }
}

/// Reads and writes one window's [`SavedTabs`]. Every call is async: the port runs it off the UI
/// thread.
#[derive(Clone)]
pub struct ClusterTabsStore {
    state: Arc<dyn StatePort>,
    table: StateTable,
    key: StateKey,
}

impl ClusterTabsStore {
    /// A store for the window `window_id` ([`MAIN_WINDOW_ID`] for the main window).
    ///
    /// # Errors
    ///
    /// `Validation` when `window_id` is not a valid state key.
    pub fn new(state: Arc<dyn StatePort>, window_id: &str) -> OxiResult<Self> {
        Ok(Self {
            state,
            table: StateTable::new(CLUSTER_TABS_TABLE)?,
            key: StateKey::new(window_id)?,
        })
    }

    /// The saved tabs, `None` when nothing was saved. A row of an unexpected shape or a newer
    /// version reads as `None` too (and is replaced by the next save): a bad row must never stop
    /// the app starting.
    ///
    /// # Errors
    ///
    /// The port's errors (database I/O).
    pub async fn load(&self) -> OxiResult<Option<SavedTabs>> {
        match self
            .state
            .table_get_as::<SavedTabs>(&self.table, &self.key)
            .await
        {
            Ok(Some(saved)) if saved.version <= CLUSTER_TABS_VERSION => Ok(Some(saved)),
            Ok(Some(saved)) => {
                tracing::warn!(
                    version = saved.version,
                    "saved cluster tabs are from a newer build: ignored"
                );
                Ok(None)
            }
            Ok(None) => Ok(None),
            Err(error) if error.kind() == oxikube_domain::ErrorKind::Validation => {
                tracing::warn!(%error, "saved cluster tabs are unreadable: ignored");
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    /// Writes `tabs`, replacing the previous row.
    ///
    /// # Errors
    ///
    /// The port's errors (database I/O).
    pub async fn save(&self, tabs: &SavedTabs) -> OxiResult<()> {
        self.state.table_put_as(&self.table, &self.key, tabs).await
    }
}

impl std::fmt::Debug for ClusterTabsStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClusterTabsStore")
            .field("window", &self.key.as_str())
            .finish_non_exhaustive()
    }
}
