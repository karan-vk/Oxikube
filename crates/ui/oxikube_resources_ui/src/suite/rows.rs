//! The table's rows under a scripted feed: initial render, add / modify / delete while scrolled,
//! sorting that stays right across deltas, and the redraw budget per delivered batch.

use gpui::TestAppContext;
use oxikube_app::ColumnId;
use oxikube_testkit::ScriptedFeed;

use super::{Scripted, numbered, pod_in};

/// The restart count shown in the Restarts cell of the row named `name` (the cell also says how
/// long ago the last one was, which moves with the clock).
fn restarts(s: &mut Scripted, name: &str) -> String {
    s.f.vcx.update(|_, cx| {
        s.table.read(cx).read_rows(cx, |d| {
            let row = d
                .rows()
                .iter()
                .find(|r| r.name() == name)
                .unwrap_or_else(|| panic!("{name} is not a row"));
            d.provider()
                .cell(row, &ColumnId::new("restarts"), jiff::Timestamp::now())
                .display()
                .split(' ')
                .next()
                .unwrap_or_default()
                .to_owned()
        })
    })
}

#[gpui::test]
fn the_initial_list_is_n_rows_and_only_the_visible_ones_are_built(cx: &mut TestAppContext) {
    let feed = ScriptedFeed::new().initial(numbered(0..2_000));
    let mut s = Scripted::open(cx, &feed);
    s.draw();
    let names = s.names();
    assert_eq!(names.len(), 2_000, "every pod of the first batch is a row");
    assert_eq!(names[0], "pod-0000");
    assert_eq!(names[1_999], "pod-1999");
    let visible =
        s.f.vcx
            .update(|_, cx| s.table.read(cx).table().visible_rows(cx));
    assert!(
        !visible.is_empty() && visible.len() < 100,
        "built {visible:?} of 2000 rows: the table is not virtualised"
    );
}

#[gpui::test]
fn add_modify_and_delete_while_scrolled_keep_the_viewport(cx: &mut TestAppContext) {
    let feed = ScriptedFeed::new()
        .initial(numbered(0..300))
        // Tick 1: a new pod at the end, a restart on a pod far below the viewport, a delete
        // below it too. Nothing happens above or inside the viewport.
        .add(1, pod_in("x", "pod-9999", 0))
        .modify(1, pod_in("x", "pod-0250", 7))
        .delete(1, pod_in("x", "pod-0299", 0))
        // Tick 2: a pod changes inside the viewport.
        .modify(2, pod_in("x", "pod-0150", 3));
    let mut s = Scripted::open(cx, &feed);
    s.f.vcx.update(|_, cx| {
        let handle = s.table.read(cx).table().clone();
        handle.scroll_to_row(150, cx);
    });
    s.draw();
    let before =
        s.f.vcx
            .update(|_, cx| s.table.read(cx).table().visible_rows(cx));
    assert!(before.contains(&150), "scrolled to row 150: {before:?}");

    s.step();
    s.draw();
    let names = s.names();
    assert_eq!(names.len(), 300, "one added, one deleted");
    assert!(names.contains(&"pod-9999".to_owned()));
    assert!(!names.contains(&"pod-0299".to_owned()));
    assert_eq!(restarts(&mut s, "pod-0250"), "7");
    let after =
        s.f.vcx
            .update(|_, cx| s.table.read(cx).table().visible_rows(cx));
    assert_eq!(after, before, "changes below the viewport do not move it");

    s.step();
    s.draw();
    assert_eq!(
        restarts(&mut s, "pod-0150"),
        "3",
        "the visible row updated in place"
    );
    let after =
        s.f.vcx
            .update(|_, cx| s.table.read(cx).table().visible_rows(cx));
    assert_eq!(
        after, before,
        "an update inside the viewport does not scroll it"
    );
    assert_eq!(s.names().len(), 300);
}

#[gpui::test]
fn sorting_stays_correct_and_stable_across_deltas(cx: &mut TestAppContext) {
    // Restarts: a=2 b=1 c=2 d=1; ties break by namespace and name.
    let feed = ScriptedFeed::new()
        .initial([
            pod_in("x", "a", 2),
            pod_in("x", "b", 1),
            pod_in("x", "c", 2),
            pod_in("x", "d", 1),
        ])
        .add(1, pod_in("x", "e", 2))
        .modify(2, pod_in("x", "b", 5))
        .delete(3, pod_in("x", "a", 2))
        .add(4, pod_in("x", "0", 1));
    let mut s = Scripted::open(cx, &feed);
    s.f.update(&s.table, |t, cx| {
        t.sort_by(Some((ColumnId::new("restarts"), false)), cx)
    });
    assert_eq!(s.names(), ["b", "d", "a", "c"], "ascending, ties by name");

    s.step();
    assert_eq!(
        s.names(),
        ["b", "d", "a", "c", "e"],
        "an equal key lands after its tie group's names"
    );
    s.step();
    assert_eq!(
        s.names(),
        ["d", "a", "c", "e", "b"],
        "a modified key moves the row, the rest keep their order"
    );
    s.step();
    assert_eq!(s.names(), ["d", "c", "e", "b"]);
    s.step();
    assert_eq!(s.names(), ["0", "d", "c", "e", "b"]);

    // Descending reverses the order of keys; the chosen sort survived every delta.
    s.f.update(&s.table, |t, cx| {
        t.sort_by(Some((ColumnId::new("restarts"), true)), cx)
    });
    assert_eq!(s.names(), ["b", "e", "c", "d", "0"]);
}

#[gpui::test]
fn a_delivered_batch_costs_at_most_one_redraw(cx: &mut TestAppContext) {
    // Tick 1 is 200 deltas in one batch; tick 2 is a single one.
    let burst = (0..100).flat_map(|i| {
        [
            oxikube_ports::Delta::Applied(pod_in("x", &format!("new-{i:03}"), 0)),
            oxikube_ports::Delta::Applied(pod_in("x", &format!("pod-{i:04}"), 1)),
        ]
    });
    let feed = ScriptedFeed::new()
        .initial(numbered(0..100))
        .at(1, burst)
        .modify(2, pod_in("x", "pod-0005", 9));
    let mut s = Scripted::open(cx, &feed);
    let burst_redraws = s.step();
    assert_eq!(s.names().len(), 200);
    assert_eq!(
        burst_redraws, 1,
        "200 deltas in a batch: one redraw, not one per delta"
    );
    let single = s.step();
    assert_eq!(single, 1, "one delta, one redraw");
    assert_eq!(restarts(&mut s, "pod-0005"), "9");
}
