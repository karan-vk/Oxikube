//! The saved list of open cluster tabs, and where each cluster's own layout is kept.
//!
//! [`ClusterTabsStore`] and [`SavedTabs`] live in `oxikube_app::session::restore` (session restore
//! reads them at launch, E06-S11, and the app layer cannot depend on this crate); they are
//! re-exported here, where the tab controller writes them. One row per window in the state table
//! [`CLUSTER_TABS_TABLE`]: cluster ids, their order, the displayed one and the names the tabs
//! showed, never credentials (non-negotiable 5). Each cluster's own pane layout is a separate row
//! in the layout table, keyed by the cluster ([`cluster_layout_key`]), so a cluster that is
//! closed and opened again gets its layout back.

use oxikube_domain::ids::ClusterId;

pub use oxikube_app::session::restore::{
    CLUSTER_TABS_TABLE, CLUSTER_TABS_VERSION, ClusterTabsStore, SavedTabs,
};

/// The layout row of a cluster's own workspace (in the table of
/// [`LayoutStore`](crate::persistence::LayoutStore)): `cluster:<id>`.
pub fn cluster_layout_key(cluster: &ClusterId) -> String {
    format!("cluster:{cluster}")
}
