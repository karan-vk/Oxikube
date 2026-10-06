//! The cluster sidebar (E06-S10): the Lens-style map of a cluster in the left dock of its tab.
//!
//! | File | Holds |
//! |---|---|
//! | `section` | [`SidebarSection`], [`SidebarEntry`], [`SidebarTarget`]: what a feature registers |
//! | `registry` | [`SidebarRegistry`]: sections come from registration, not from the shell |
//! | `core` | the eleven core sections as placeholders ([`core_sections`]) |
//! | `rows` | the flat row list and the visibility rules ([`build_rows`]), pure |
//! | `panel` | [`SidebarPanel`]: the dock panel that follows the session and draws the rows |
//! | `store` | [`SidebarStore`]: open and closed groups, saved per cluster in the `StatePort` |
//! | `actions` | the panel's key bindings |
//!
//! # Sections and visibility
//!
//! Cluster, Nodes, Workloads, Config, Network, Storage, Namespaces, Events, Helm, Access Control
//! and Custom Resources, each a collapsible group with a count placeholder (the `ResourceStore`
//! fills counts later). A section shows when the user may `list` at least one of the kinds it
//! covers, judged by the `SelfSubjectRulesReview` of the selected namespaces
//! ([`review_access`](oxikube_app::sidebar::review_access)); the review runs after the session
//! is `Ready` and again when the namespace selection changes or the cluster reconnects, and until
//! it answers only sections that need no access (the overview) show. A failed review fails open:
//! everything shows, with a warning. When sections are hidden a muted line says so. Custom
//! Resources lists the non-built-in API groups from discovery and hides when none is listable.
//!
//! Integrations append their sections after the core ones, in registration order
//! ([`IntegrationRegistry`](oxikube_app::IntegrationRegistry)).
//!
//! # Adding a section
//!
//! From a crate's `init(cx)`: `SidebarRegistry::register(cx, SidebarSection::new(..))`, or
//! `SidebarRegistry::add_entry(cx, "workloads", entry)` for one more kind in an existing section.
//! Every open sidebar redraws.
//!
//! Section clicks navigate (pure UI, [`SidebarEvent::Navigate`]); nothing here mutates a cluster.

mod actions;
mod core;
mod panel;
mod registry;
mod rows;
mod section;
mod store;

#[cfg(test)]
mod tests;

pub use actions::{Activate, Collapse, Expand, MoveDown, MoveUp, SIDEBAR_CONTEXT, Toggle};
pub use core::{ORDER_STEP, core_sections, register_core_sections};
pub use panel::{DEFAULT_WIDTH, SidebarDeps, SidebarEvent, SidebarPanel};
pub use registry::SidebarRegistry;
pub use rows::{
    AccessState, EntryRow, GroupRow, NoticeKind, NoticeRow, Row, RowInputs, SectionRow, build_rows,
};
pub use section::{SectionBody, SidebarEntry, SidebarSection, SidebarTarget};
pub use store::{SIDEBAR_TABLE, SIDEBAR_VERSION, SavedSidebar, SidebarStore};

use gpui::{App, AppContext as _, Entity, Window};
use oxikube_app::ClusterSession;

use crate::cluster_tab::ClusterTab;

/// Registers the core sections and the sidebar's key bindings. Called by [`crate::init`].
pub fn init(cx: &mut App) {
    register_core_sections(cx);
    actions::register(cx);
}

impl SidebarPanel {
    /// The sidebar of `cluster`, following its session through `deps`.
    pub fn build(
        cluster: oxikube_domain::ids::ClusterId,
        deps: SidebarDeps,
        cx: &mut App,
    ) -> Entity<Self> {
        cx.new(|cx| Self::new_in(cluster, deps, cx))
    }
}

/// The setup hook that gives every cluster tab its sidebar: pass it to
/// [`ClusterTabsDeps::with_setup`](crate::ClusterTabsDeps::with_setup) (or call it from your own
/// hook). Adds a [`SidebarPanel`] to the tab's workspace, in the left dock.
pub fn tab_setup(
    deps: SidebarDeps,
) -> impl Fn(&Entity<ClusterTab>, &ClusterSession, &mut Window, &mut App) + 'static {
    move |tab, session, window, cx| {
        let panel = SidebarPanel::build(session.id().clone(), deps.clone(), cx);
        let workspace = tab.read(cx).workspace().clone();
        workspace.update(cx, |ws, cx| ws.add_panel(panel, window, cx));
    }
}
