//! Column layout per kind: reorder, resize, hide and sort persist through the `StatePort` and
//! come back when the view is recreated.

use gpui::{AppContext as _, TestAppContext};
use oxikube_app::ColumnId;
use oxikube_ports::StatePort as _;
use oxikube_ui::Unscaled;
use oxikube_ui::table::TableEvent;

use super::fixture::{Fixture, cluster};
use super::{p, pods_kind};
use crate::table::{ColumnPrefs, ResourceTable, prefs_key};

#[gpui::test]
fn reorder_resize_hide_and_sort_persist_per_kind(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.connect_with([p("x", "a", "1"), p("x", "b", "1")]);
    let table = f.open_pods();
    let before = f
        .vcx
        .update(|_, cx| table.read(cx).read_rows(cx, |d| d.layout().visible_ids()));
    assert!(before.contains(&ColumnId::new("node")));

    // Hide a column, drag the third to the front, widen the name, sort by restarts.
    f.update(&table, |t, cx| {
        t.set_column_shown(&ColumnId::new("node"), false, cx)
    });
    f.update(&table, |t, cx| {
        t.table().update(cx, |d| d.layout.move_visible(2, 0));
        t.on_table_event(&TableEvent::ColumnMoved { from: 2, to: 0 }, cx);
    });
    let widths: Vec<Unscaled> = f.vcx.update(|_, cx| {
        table.read(cx).read_rows(cx, |d| {
            (0..d.layout().visible_len())
                .map(|ix| {
                    let column = d.layout().visible(ix).unwrap();
                    let width = crate::table::ColumnLayout::default_width(column);
                    Unscaled(if column.id == ColumnId::NAME {
                        333.
                    } else {
                        width
                    })
                })
                .collect()
        })
    });
    f.update(&table, |t, cx| {
        t.on_table_event(&TableEvent::ColumnsResized(widths), cx)
    });
    f.update(&table, |t, cx| {
        t.sort_by(Some((ColumnId::new("restarts"), true)), cx)
    });
    let arranged = f
        .vcx
        .update(|_, cx| table.read(cx).read_rows(cx, |d| d.layout().clone()));
    assert!(!arranged.is_shown(&ColumnId::new("node")));
    assert_eq!(arranged.visible_ids()[0], before[2]);

    // It is in the state store under the kind's key.
    let key = prefs_key(&pods_kind().gvk).unwrap();
    let saved: ColumnPrefs = futures::executor::block_on(f.state.kv_get(&key))
        .unwrap()
        .map(|v| serde_json::from_value(v).unwrap())
        .expect("the layout was saved");
    assert_eq!(saved.visible.get("node"), Some(&false));
    assert_eq!(saved.widths.get("name"), Some(&333.));
    assert_eq!(
        saved
            .sort
            .as_ref()
            .map(|s| (s.column.as_str(), s.descending)),
        Some(("restarts", true))
    );

    // A new view of the kind reads it back.
    let deps = f.deps.clone();
    let again = f.vcx.update(|window, cx| {
        cx.new(|cx| ResourceTable::new(cluster(), pods_kind(), deps, window, cx))
    });
    f.settle();
    let restored = f
        .vcx
        .update(|_, cx| again.read(cx).read_rows(cx, |d| d.layout().clone()));
    assert_eq!(restored, arranged);
    // Restarts tie at 0, so the descending sort reverses the tie-break on the name.
    assert_eq!(
        f.names(&again),
        ["b", "a"],
        "and lists the kind in the saved order"
    );
}

#[gpui::test]
fn the_defaults_show_until_the_saved_layout_is_read(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.connect_with([p("x", "a", "1")]);
    let table = f.open_pods();
    // Nothing saved: opening and reading writes nothing over it.
    let key = prefs_key(&pods_kind().gvk).unwrap();
    assert!(
        futures::executor::block_on(f.state.kv_get(&key))
            .unwrap()
            .is_none()
    );
    let shown = f.vcx.update(|_, cx| {
        table
            .read(cx)
            .read_rows(cx, |d| d.layout().is_shown(&ColumnId::new("qos")))
    });
    assert!(!shown, "wide columns start hidden");
}

/// The sort column of `table`'s subscription, if it sorts by a column.
fn subscription_sort(
    f: &mut Fixture,
    table: &gpui::Entity<ResourceTable>,
) -> Option<(String, bool)> {
    f.vcx.update(|_, cx| {
        let query = table.read(cx).subscription.as_ref()?.query().clone();
        match query.sort.field {
            oxikube_app::store::SortField::Cell(key) => {
                Some((key.column.to_string(), query.sort.descending))
            }
            _ => None,
        }
    })
}

#[gpui::test]
fn the_saved_sort_and_order_survive_until_the_feed_columns_arrive(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    // Saved earlier: status first, sorted by restarts, descending.
    let key = prefs_key(&pods_kind().gvk).unwrap();
    let saved = ColumnPrefs {
        order: vec!["status".into(), "name".into()],
        sort: Some(crate::table::SavedSort {
            column: "restarts".into(),
            descending: true,
        }),
        ..ColumnPrefs::default()
    };
    futures::executor::block_on(f.state.kv_set(&key, serde_json::to_value(&saved).unwrap()))
        .unwrap();
    f.connect_with([p("x", "a", "1"), p("x", "b", "1")]);
    let table = f.open_pods();
    let full = f
        .vcx
        .update(|_, cx| table.read(cx).read_rows(cx, |d| d.layout().clone()));
    assert_eq!(full.sort(), Some(&(ColumnId::new("restarts"), true)));

    // The layout read before the feed's columns: only the generic ones (a Table-feed kind).
    let generic: std::sync::Arc<dyn oxikube_app::ColumnProvider> =
        std::sync::Arc::new(oxikube_app::TableColumns::new(
            &[],
            oxikube_ports::TableSource::Objects,
            pods_kind().scope(),
        ));
    f.update(&table, |t, cx| t.set_provider(generic, cx));
    assert_eq!(
        subscription_sort(&mut f, &table),
        None,
        "no restarts column"
    );
    // A save meanwhile (a resize) keeps the choices for the absent columns.
    f.update(&table, |t, cx| t.save_prefs(cx));
    let stored: ColumnPrefs = futures::executor::block_on(f.state.kv_get(&key))
        .unwrap()
        .map(|v| serde_json::from_value(v).unwrap())
        .unwrap();
    assert_eq!(stored.sort, saved.sort);
    assert_eq!(&stored.order[..2], ["status", "name"]);

    // The feed's columns arrive: the saved sort and order are back, and the store sorts by them.
    let core: std::sync::Arc<dyn oxikube_app::ColumnProvider> = f.deps.columns.clone();
    f.update(&table, |t, cx| t.set_provider(core, cx));
    let restored = f
        .vcx
        .update(|_, cx| table.read(cx).read_rows(cx, |d| d.layout().clone()));
    assert_eq!(restored, full);
    assert_eq!(restored.visible_ids()[0], ColumnId::new("status"));
    assert_eq!(
        subscription_sort(&mut f, &table),
        Some(("restarts".to_owned(), true))
    );
    assert_eq!(f.names(&table), ["b", "a"]);
}
