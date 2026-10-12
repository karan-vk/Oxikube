//! Tests of the kind registry's UI half: registration and publishing (`registry`), the cluster
//! sidebar built from it over discovery sets (`sidebar`), a kind's own columns and detail panel
//! reaching the table and the drawer (`spec`), the CRD kind's panel (`crd`, E12-S17). `#[gpui::test]`s over testkit fakes.

mod batch;
mod configmap;
mod crd;
mod network;
mod registry;
mod secret;
mod sidebar;
mod spec;

use std::sync::Arc;

use gpui::Entity;
use jiff::Timestamp;
use oxikube_app::store::StoreObject;
use oxikube_app::{Cell, Column, ColumnId, ColumnProvider, KindMeta, Section};
use oxikube_domain::Capabilities;
use oxikube_domain::ids::{Gvk, Scope};
use oxikube_workspace::sidebar::SidebarPanel;

use crate::table::tests::fixture::{Fixture, cluster};

/// The sidebar of the fixture's cluster tab.
pub(super) fn sidebar_panel(f: &mut Fixture) -> Entity<SidebarPanel> {
    let tabs = f.tabs.clone();
    f.vcx.update(|_, cx| {
        let tab = tabs.read(cx).tab(&cluster()).cloned().expect("a tab");
        let workspace = tab.read(cx).workspace().clone();
        workspace
            .read(cx)
            .panel::<SidebarPanel>()
            .expect("the tab has its sidebar")
    })
}

/// The ids of the sidebar's rows, in order.
pub(super) fn row_ids(f: &mut Fixture) -> Vec<String> {
    let panel = sidebar_panel(f);
    f.vcx.update(|_, cx| {
        panel
            .read(cx)
            .rows()
            .iter()
            .map(|r| r.id().to_owned())
            .collect()
    })
}

/// A provider with two columns of its own, `Name` and `Phase`.
pub(super) struct PodColumns;

impl PodColumns {
    pub(super) const PHASE: &'static str = "kind-phase";
}

impl ColumnProvider for PodColumns {
    fn columns(&self, _: &Gvk, _: Capabilities) -> Arc<[Column]> {
        let column = |id: &'static str, title: &str| Column {
            id: ColumnId::new(id),
            title: Arc::from(title),
            description: None,
            wide: false,
            align: oxikube_app::columns::Align::Left,
            sort: oxikube_app::columns::SortKind::Text,
            table_index: None,
            sortable: true,
        };
        Arc::new([column(ColumnId::NAME, "Name"), column(Self::PHASE, "Phase")])
    }

    fn cell<'a>(&self, object: &'a StoreObject, column: &ColumnId, _: Timestamp) -> Cell<'a> {
        match column.as_str() {
            ColumnId::NAME => Cell::text(object.name()),
            _ => Cell::text("Running"),
        }
    }
}

/// The Pods spec with [`PodColumns`].
pub(super) fn pods_with_own_columns() -> KindMeta {
    KindMeta::new(Gvk::new("", "v1", "Pod"), "pods", "Pods", Scope::Namespaced)
        .in_section(Section::WORKLOADS, 10)
        .with_columns(Arc::new(PodColumns))
}
