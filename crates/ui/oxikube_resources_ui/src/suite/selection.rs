//! Selection and the cursor follow objects, not row numbers: they survive adds above, modified
//! versions, re-sorts and relists, and drop an object only when it is gone.

use gpui::{Modifiers, TestAppContext};
use oxikube_app::ColumnId;
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_ports::Delta;
use oxikube_testkit::ScriptedFeed;

use super::{Scripted, pod_in};
use crate::table::tests::fixture::cluster;

/// Clicks the first cell of `row` (with the platform's toggle modifier when `toggle`).
fn click(s: &mut Scripted, row: usize, toggle: bool) {
    s.draw();
    let at =
        s.f.vcx
            .debug_bounds(Box::leak(format!("cell-{row}-0").into_boxed_str()))
            .unwrap_or_else(|| panic!("row {row} was not laid out"))
            .center();
    let modifiers = if toggle {
        Modifiers::secondary_key()
    } else {
        Modifiers::none()
    };
    s.f.vcx.simulate_click(at, modifiers);
    s.f.settle();
}

fn selected(s: &mut Scripted) -> Vec<String> {
    s.f.selected(&s.table)
}

/// The rows of the table that are highlighted, as indices.
fn highlighted(s: &mut Scripted) -> Vec<usize> {
    s.f.vcx.update(|_, cx| {
        s.table.read(cx).read_rows(cx, |d| {
            (0..d.rows().len())
                .filter(|&i| d.selection().contains_object(&d.rows()[i]))
                .collect()
        })
    })
}

#[gpui::test]
fn the_selection_survives_adds_modifies_and_relists_by_resource_ref(cx: &mut TestAppContext) {
    let feed = ScriptedFeed::new()
        .initial([
            pod_in("x", "b", 0),
            pod_in("x", "c", 0),
            pod_in("x", "d", 0),
        ])
        // Tick 1: a pod lands above the selection, a selected pod gets a new version.
        .add(1, pod_in("x", "a", 0))
        .modify(1, pod_in("x", "c", 4))
        // Tick 2: a selected pod is deleted.
        .delete(2, pod_in("x", "d", 0))
        // Tick 3: the deleted pod comes back (a new object with the old name).
        .add(3, pod_in("x", "d", 0))
        // Tick 4: the feed relists (a reconnect): `b` and `c` are still there, `e` is new.
        .at(
            4,
            [Delta::Restarted(vec![
                pod_in("x", "b", 0),
                pod_in("x", "c", 4),
                pod_in("x", "e", 0),
            ])],
        );
    let mut s = Scripted::open(cx, &feed);
    click(&mut s, 0, false); // b
    click(&mut s, 2, true); // d
    click(&mut s, 1, true); // c
    assert_eq!(selected(&mut s), ["b", "c", "d"]);

    s.step();
    assert_eq!(s.names(), ["a", "b", "c", "d"]);
    assert_eq!(
        selected(&mut s),
        ["b", "c", "d"],
        "the pod above and the new version change nothing"
    );
    assert_eq!(
        highlighted(&mut s),
        [1, 2, 3],
        "the highlight moved down with the rows"
    );

    s.step();
    assert_eq!(
        selected(&mut s),
        ["b", "c"],
        "the deleted pod left the selection"
    );

    s.step();
    assert_eq!(s.names(), ["a", "b", "c", "d"]);
    assert_eq!(
        selected(&mut s),
        ["b", "c"],
        "a pod that left the selection with its row is not selected again when it returns"
    );

    s.step();
    assert_eq!(s.names(), ["b", "c", "e"]);
    assert_eq!(
        selected(&mut s),
        ["b", "c"],
        "a relist keeps what is still there"
    );
    assert_eq!(highlighted(&mut s), [0, 1]);
}

#[gpui::test]
fn the_selected_refs_are_what_actions_and_the_detail_receive(cx: &mut TestAppContext) {
    let feed = ScriptedFeed::new()
        .initial([pod_in("x", "a", 0), pod_in("y", "a", 0)])
        .add(1, pod_in("w", "a", 0));
    let mut s = Scripted::open(cx, &feed);
    click(&mut s, 1, false); // y/a: same name as x/a, other namespace
    s.step();
    // A row above arrived; the object is still `y/a`, not whatever now sits at row 1.
    let refs: Vec<ResourceRef> = s.f.vcx.update(|_, cx| s.table.read(cx).selected_refs(cx));
    assert_eq!(
        refs,
        [ResourceRef::namespaced(
            cluster(),
            Gvk::new("", "v1", "Pod"),
            "y",
            "a"
        )]
    );
    assert_eq!(s.names(), ["a", "a", "a"], "three pods named a");
    assert_eq!(
        highlighted(&mut s),
        [2],
        "namespace is part of the identity"
    );
}

#[gpui::test]
fn a_resort_keeps_the_selection_and_the_cursor_on_the_same_objects(cx: &mut TestAppContext) {
    let feed = ScriptedFeed::new()
        .initial([
            pod_in("x", "a", 3),
            pod_in("x", "b", 1),
            pod_in("x", "c", 2),
        ])
        .modify(1, pod_in("x", "b", 9));
    let mut s = Scripted::open(cx, &feed);
    s.f.keys(&s.table, "j j"); // the first `j` lands on row 0, the second moves to b
    assert_eq!(selected(&mut s), ["b"]);
    s.f.update(&s.table, |t, cx| {
        t.sort_by(Some((ColumnId::new("restarts"), false)), cx)
    });
    assert_eq!(s.names(), ["b", "c", "a"]);
    assert_eq!(selected(&mut s), ["b"]);
    s.step();
    assert_eq!(
        s.names(),
        ["c", "a", "b"],
        "b's new count moved it to the end"
    );
    assert_eq!(selected(&mut s), ["b"], "and the selection went with it");
    assert_eq!(highlighted(&mut s), [2]);
    let cursor = s.f.vcx.update(|_, cx| s.table.read(cx).cursor_row(cx));
    assert_eq!(cursor, Some(2), "the cursor too");
}
