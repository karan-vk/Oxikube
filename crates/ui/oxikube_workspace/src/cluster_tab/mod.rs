//! Cluster tabs (E06-S04): one workspace tab per live cluster session.
//!
//! The window's [`Workspace`](crate::Workspace) holds the catalog and, next to it, a
//! [`ClusterTab`] for every cluster whose session is not disconnected. A cluster tab is an
//! [`Item`](crate::Item) that hosts a workspace of its own (see [`ClusterTab`] for why): its own
//! docks (the sidebar panel, E06-S10) and pane group (the cluster's resource views, E07), laid
//! out and saved per cluster, independent of every other cluster's.
//!
//! | Module | Holds |
//! |---|---|
//! | `tab` | [`ClusterTab`]: the item, its tab label (colour dot, lock, title) and stripe |
//! | `controller` | [`ClusterTabs`]: follows the sessions, switches, closes, saves the tab list |
//! | `dispatch` | [`CommandDispatcher`], [`CommandSink`], [`TabsDispatcher`] |
//! | `actions` | the `cluster::SwitchTab`, `NextTab` and `PreviousTab` actions behind `cmd-1..9` |
//! | `store` | [`ClusterTabsStore`]: the saved list of open tabs |
//! | `colour` | the cluster colour as a GPUI colour, and initials |
//!
//! # Commands
//!
//! `cluster::Select` (the user's "switch"), `cluster::SwitchTab`, `cluster::NextTab`,
//! `cluster::PreviousTab` and `cluster::CloseTab` are declared in `oxikube_domain::command`, so
//! each has an MCP tool stub, and [`register_commands`] puts them on the `CommandBus`. The key
//! bindings in the per-OS keymaps (`cmd-1` to `cmd-9`, `ctrl-shift-1` to `ctrl-shift-9` on
//! Linux and Windows, `ctrl-tab`) reach them through the actions in [`actions`]. None changes a cluster,
//! so none goes through `MutationGuard`; closing a tab sends `cluster::Disconnect`.
//!
//! # Saved state
//!
//! - which clusters are open, in what order, which is displayed: [`ClusterTabsStore`], one row
//!   per window;
//! - each cluster's own pane layout: the layout table, keyed `cluster:<id>` (see
//!   [`cluster_layout_key`]), so closing a cluster and opening it again gives its layout back.
//!
//! # While the cluster is not connected
//!
//! A tab opens when its session starts connecting, so it is where the connect lifecycle shows
//! (E06-S06). The tab hosts a [`ConnectUi`] when its owner sets one
//! ([`ClusterTab::set_connect_ui`]; `oxikube_catalog_ui::connect` builds it): its body replaces
//! the content while the session is connecting, needs credentials or failed, and its banner sits
//! above the content while the session is degraded. Without one the tab shows a plain
//! placeholder.
//!
//! # Session restore
//!
//! With `session.restore` on, [`ClusterTabs::restore_session`] reopens the saved clusters after
//! the first frame as placeholder tabs and connects the displayed one (`session.restore_connect`
//! says whether the others connect too, or when their tab is first shown): see the `controller`
//! module's `restore` file. The decisions (which clusters still exist, namespaces, limits and
//! timeouts) are `oxikube_app::session::restore`.

pub mod actions;
mod colour;
mod controller;
mod dispatch;
mod restore_settings;
mod store;
mod tab;

#[cfg(test)]
mod tests;

pub use colour::{cluster_hsla, initials};
pub use controller::{ClusterTabs, ClusterTabsDeps, ClusterTabsEvent, TabSetup, register_commands};
pub use dispatch::{CommandDispatcher, CommandSink, TabsDispatcher, is_tab_command};
pub use restore_settings::{RestoreConnectSetting, SessionRestoreContent, SessionRestoreSettings};
pub use store::{
    CLUSTER_TABS_TABLE, CLUSTER_TABS_VERSION, ClusterTabsStore, SavedTabs, cluster_layout_key,
};
pub use tab::{ClusterTab, ClusterTabEvent, ClusterTabInfo, ConnectUi};

/// Registers the cluster-tab actions and their key handlers. Called by [`crate::init`].
pub fn init(cx: &mut gpui::App) {
    actions::register(cx);
}
