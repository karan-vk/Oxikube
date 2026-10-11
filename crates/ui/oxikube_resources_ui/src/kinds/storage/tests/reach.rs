//! The storage kinds are reachable from the cluster tab's sidebar, and open as tables and details.

use gpui::TestAppContext;
use oxikube_domain::access::AccessRules;
use oxikube_testkit::discovery_sets;
use oxikube_testkit::storage::{pvc, storage_class};

use super::{connect, kind, open_detail, storage_window};
use crate::kinds::tests::{row_ids, sidebar_panel};
use crate::table::ResourceTable;
use crate::table::tests::fixture::{Fixture, cluster};

fn storage_rows(ids: &[String]) -> Vec<&str> {
    ids.iter()
        .map(String::as_str)
        .filter(|id| id.starts_with("storage/"))
        .collect()
}

#[gpui::test]
fn the_storage_section_lists_claims_volumes_and_classes_and_snapshots_where_served(
    cx: &mut TestAppContext,
) {
    let mut f = storage_window(cx, Vec::new());
    f.ports().access.set_rules(AccessRules::all_access());
    f.settle();
    let ids = row_ids(&mut f);
    assert_eq!(
        storage_rows(&ids),
        [
            "storage/persistentvolumeclaims",
            "storage/persistentvolumes",
            "storage/storageclasses",
            "storage/volumesnapshots",
            "storage/volumesnapshotclasses",
        ]
    );
}

#[gpui::test]
fn a_cluster_without_the_snapshot_crds_does_not_show_them(cx: &mut TestAppContext) {
    let mut f = Fixture::with_storage(cx);
    f.vcx.update(|_, cx| crate::kinds::init(cx));
    connect(&mut f, discovery_sets::core(), Vec::new());
    f.ports().access.set_rules(AccessRules::all_access());
    f.settle();
    let ids = row_ids(&mut f);
    assert_eq!(
        storage_rows(&ids),
        [
            "storage/persistentvolumeclaims",
            "storage/persistentvolumes",
            "storage/storageclasses",
        ],
        "VolumeSnapshots need their CRDs"
    );
}

#[gpui::test]
fn a_sidebar_entry_opens_the_claims_table(cx: &mut TestAppContext) {
    let mut f = storage_window(
        cx,
        vec![
            pvc("data").namespace("shop").build(),
            storage_class("standard").build(),
        ],
    );
    let panel = sidebar_panel(&mut f);
    f.vcx.update(|_, cx| {
        panel.update(cx, |panel, cx| {
            panel.activate("storage/persistentvolumeclaims", cx)
        })
    });
    f.settle();
    let tabs = f.tabs.clone();
    let tables: Vec<gpui::Entity<ResourceTable>> = f.vcx.update(|_, cx| {
        let tab = tabs.read(cx).tab(&cluster()).cloned().expect("a tab");
        tab.read(cx)
            .workspace()
            .read(cx)
            .items_of_type::<ResourceTable>()
    });
    assert_eq!(tables.len(), 1, "the entry opened the claims table");
    assert_eq!(f.names(&tables[0]), ["data"]);
}

#[gpui::test]
fn every_storage_kind_opens_a_detail_with_its_panel(cx: &mut TestAppContext) {
    use oxikube_domain::ids::ResourceRef;
    let mut f = storage_window(
        cx,
        vec![
            pvc("data").namespace("shop").class("standard").build(),
            storage_class("standard").build(),
        ],
    );
    let claim = ResourceRef::namespaced(
        cluster(),
        kind("persistentvolumeclaims").gvk,
        "shop",
        "data",
    );
    let class = ResourceRef::cluster_scoped(cluster(), kind("storageclasses").gvk, "standard");
    for target in [claim, class] {
        let view = open_detail(&mut f, &target);
        f.vcx.update(|window, cx| window.draw(cx).clear(cx));
        let has_panel = f.vcx.update(|_, cx| view.read(cx).kind_panel().is_some());
        assert!(has_panel, "{} has a panel", target.gvk.kind);
        assert!(f.vcx.debug_bounds("storage-panel").is_some());
    }
}
