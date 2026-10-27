//! The RBAC subject pages (E17-S07): who the cluster's bindings name, and what the policy grants
//! each of them.
//!
//! * **Users** (`:usr`, the sidebar's Access Control > Users): one row per user and ServiceAccount
//!   some RoleBinding or ClusterRoleBinding names, with the number of bindings and roles. A
//!   ServiceAccount is listed under its `system:serviceaccount:<ns>:<name>` identity.
//! * **Groups** (`:grp`, the sidebar's Groups): the same for groups. The API cannot list a
//!   group's members, so the page says so and shows what the bindings grant to the group.
//! * **A subject's matrix** (`:rbac <subject>`, Enter on a row): k9s's policy table. Rows are API
//!   groups and resources (non-resource URLs after them), columns `GET LIST WATCH CREATE PATCH
//!   UPDATE DELETE DEL-LIST EXTRAS`, each cell a tick (every object), `~` (only named
//!   `resourceNames`) or a dash. A scope toggle picks cluster-wide or one namespace, the
//!   implicit-groups toggle adds what `system:authenticated` and a ServiceAccount's groups give,
//!   and selecting a row lists, per verb column, the bindings and roles that grant the cell.
//!   `rbac::CopyMatrix` copies the matrix as tab-separated text.
//!
//! | File | Holds |
//! |---|---|
//! | `view` | [`SubjectsView`]: the entity (an item of the cluster's tab), state, actions, the background reads |
//! | `render` | the toolbar, the notes, the tables and the provenance detail |
//! | `list`, `grid` | [`ListDelegate`], [`MatrixDelegate`]: the two virtualised tables' rows, filter and selection |
//! | `backend` | [`SubjectsBackend`]: [`StoreBackend`] over the cluster's store, [`StaticBackend`] for fixtures |
//! | `open` | the `rbac::*` commands, [`SubjectsRequestSink`], [`SubjectsViews`] (the window's controller) |
//! | `actions`, `model` | the `RbacSubjects` key actions; [`Page`], [`Phase`] |
//!
//! # Performance
//!
//! Both tables are [`oxikube_ui::Table`]s (a `uniform_list`: only the rows on screen are built);
//! cell texts are built once per read and the filter works on indices. The policy arrives through
//! the store's shared feeds, the index and every matrix and provenance are computed on the
//! background executor, and a burst of binding events is one read ([`SETTLE`]). Redraws from the
//! background go through `notify_coalesced`.

mod actions;
mod backend;
mod detail;
mod grid;
mod list;
mod matrix_actions;
mod model;
mod open;
mod read;
mod render;
mod selection;
mod view;

#[cfg(test)]
mod tests;

use gpui::{App, Context, SharedString, Window};
use oxikube_ui::IconName;
use oxikube_workspace::{Item, TabContent, Workspace};

pub use actions::{
    Back, CopyMatrix, FocusFilter, FocusTable, NextColumn, NextScope, OpenSelected, PreviousColumn,
    SelectFirst, SelectLast, SelectNext, SelectPrevious, ToggleImplicitGroups,
};
pub use backend::{
    PolicyWatch, StaticBackend, StoreBackend, StoreLookup, SubjectsBackend, guess_subject,
};
pub use grid::{MatrixDelegate, mark};
pub use list::ListDelegate;
pub use model::{Page, Phase, scope_label};
pub use open::{
    Backends, Change, RBAC_COMMANDS, RbacRequest, SubjectsHost, SubjectsRequestSink, SubjectsViews,
    SubjectsViewsDeps, register_commands,
};
pub use view::{SETTLE, SubjectsDeps, SubjectsView};

/// The deduplication key of the tab: opening a subject page twice shows the open one.
pub const ITEM_KEY: &str = "cluster-rbac-subjects";

impl SubjectsView {
    /// The subject page open in `workspace`, if any.
    pub fn find(
        workspace: &gpui::Entity<Workspace>,
        cx: &App,
    ) -> Option<gpui::Entity<SubjectsView>> {
        workspace
            .read(cx)
            .items_of_type::<SubjectsView>()
            .into_iter()
            .next()
    }
}

impl Item for SubjectsView {
    fn tab_content(&self, _: &App) -> TabContent {
        TabContent::new(self.title()).icon(IconName::ShieldCheck)
    }

    fn item_key(&self, _: &App) -> Option<SharedString> {
        Some(ITEM_KEY.into())
    }

    fn on_close(&mut self, _: &mut Window, _: &mut Context<Self>) {}
}
