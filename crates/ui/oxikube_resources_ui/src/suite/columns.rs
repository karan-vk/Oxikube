//! Column management end to end: hide, reorder, resize and sort a kind's table, then recreate the
//! view (what reopening the tab or restarting the app does) and find the same layout, applied to
//! a live feed.

use futures::executor::block_on;
use gpui::{AppContext as _, TestAppContext};
use oxikube_app::ColumnId;
use oxikube_ports::StatePort as _;
use oxikube_testkit::ScriptedFeed;
use oxikube_ui::Unscaled;
use oxikube_ui::table::TableEvent;

use super::{Scripted, pod_in};
use crate::table::tests::fixture::{Fixture, cluster};
use crate::table::tests::pods_kind;
use crate::table::{ColumnLayout, ColumnPrefs, ResourceTable, prefs_key};

fn layout(f: &mut Fixture, table: &gpui::Entity<ResourceTable>) -> ColumnLayout {
    f.vcx
        .update(|_, cx| table.read(cx).read_rows(cx, |d| d.layout().clone()))
}

/// The column preferences saved under `key` in the state store.
fn saved_prefs(f: &mut Fixture, key: &oxikube_ports::StateKey) -> ColumnPrefs {
    block_on(f.state.kv_get(key))
        .expect("state")
        .map(|v| serde_json::from_value(v).expect("prefs"))
        .expect("the layout was saved")
}

#[gpui::test]
fn hide_reorder_resize_and_sort_persist_across_view_recreation(cx: &mut TestAppContext) {
    let feed = ScriptedFeed::new()
        .initial([
            pod_in("x", "a", 1),
            pod_in("x", "b", 5),
            pod_in("x", "c", 3),
        ])
        .add(1, pod_in("x", "d", 4))
        .modify(2, pod_in("x", "a", 9));
    let mut s = Scripted::open(cx, &feed);
    let defaults = layout(&mut s.f, &s.table).visible_ids();

    // Hide Node, drag the third column to the front, widen the name, sort by restarts descending.
    s.f.update(&s.table, |t, cx| {
        t.set_column_shown(&ColumnId::new("node"), false, cx)
    });
    s.f.update(&s.table, |t, cx| {
        t.table().update(cx, |d| d.layout.move_visible(2, 0));
        t.on_table_event(&TableEvent::ColumnMoved { from: 2, to: 0 }, cx);
    });
    let widths: Vec<Unscaled> = s.f.vcx.update(|_, cx| {
        s.table.read(cx).read_rows(cx, |d| {
            (0..d.layout().visible_len())
                .map(|ix| {
                    let column = d.layout().visible(ix).expect("a visible column");
                    Unscaled(if column.id == ColumnId::NAME {
                        321.
                    } else {
                        ColumnLayout::default_width(column)
                    })
                })
                .collect()
        })
    });
    s.f.update(&s.table, |t, cx| {
        t.on_table_event(&TableEvent::ColumnsResized(widths), cx)
    });
    s.f.update(&s.table, |t, cx| {
        t.sort_by(Some((ColumnId::new("restarts"), true)), cx)
    });
    let arranged = layout(&mut s.f, &s.table);
    assert!(!arranged.is_shown(&ColumnId::new("node")));
    assert_eq!(arranged.visible_ids()[0], defaults[2], "reordered");

    // It is in the state store under the kind's key, nowhere else.
    let key = prefs_key(&pods_kind().gvk).expect("a key");
    assert_eq!(
        key.as_str(),
        "table.columns.core/Pod",
        "the documented per-kind key: a change loses every user's saved layouts"
    );
    let saved = saved_prefs(&mut s.f, &key);
    assert_eq!(saved.visible.get("node"), Some(&false));
    assert_eq!(saved.widths.get("name"), Some(&321.));

    // A recreated view of the kind reads it back and applies it to the live feed.
    let deps = s.f.deps.clone();
    let again = s.f.vcx.update(|window, cx| {
        cx.new(|cx| ResourceTable::new(cluster(), pods_kind(), deps, window, cx))
    });
    s.f.settle();
    assert_eq!(
        layout(&mut s.f, &again),
        arranged,
        "same order, widths, visibility, sort"
    );
    assert_eq!(
        s.f.names(&again),
        ["b", "c", "a"],
        "restarts, highest first"
    );

    // Both views follow the feed in the saved order; the hidden column stays hidden.
    s.step();
    assert_eq!(s.f.names(&again), ["b", "d", "c", "a"]);
    s.step();
    assert_eq!(
        s.f.names(&again),
        ["a", "b", "d", "c"],
        "a has 9 restarts now"
    );
    assert_eq!(s.names(), s.f.names(&again), "the first view agrees");
    assert!(!layout(&mut s.f, &again).is_shown(&ColumnId::new("node")));
}

#[gpui::test]
fn a_column_shown_and_hidden_again_is_saved_and_then_forgotten(cx: &mut TestAppContext) {
    let feed = ScriptedFeed::new().initial([pod_in("x", "a", 0)]);
    let mut s = Scripted::open(cx, &feed);
    let key = prefs_key(&pods_kind().gvk).expect("a key");
    s.f.update(&s.table, |t, cx| {
        t.set_column_shown(&ColumnId::new("qos"), true, cx)
    });
    assert_eq!(
        saved_prefs(&mut s.f, &key).visible.get("qos"),
        Some(&true),
        "a wide column shown"
    );
    s.f.update(&s.table, |t, cx| {
        t.set_column_shown(&ColumnId::new("qos"), false, cx)
    });
    assert_eq!(
        saved_prefs(&mut s.f, &key).visible.get("qos"),
        None,
        "hidden again is the default: the override is gone, not a stale `true`"
    );
}
