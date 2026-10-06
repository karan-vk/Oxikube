//! [`Selection`]: clicks, ranges, toggles, keys, and deltas that move rows under it.

use std::sync::Arc;

use oxikube_app::store::{RowOp, StoreObject};

use super::{key, p, rows};
use crate::table::{ClickMode, Selection};

fn selected(selection: &Selection, rows: &[Arc<StoreObject>]) -> Vec<String> {
    selection
        .in_row_order(rows)
        .into_iter()
        .map(|k| k.name.to_string())
        .collect()
}

#[test]
fn click_shift_range_and_toggle() {
    let list = rows(&["a", "b", "c", "d", "e"]);
    let mut s = Selection::default();
    s.click(&list, 1, ClickMode::Replace);
    assert_eq!(selected(&s, &list), ["b"]);
    s.click(&list, 3, ClickMode::Extend);
    assert_eq!(
        selected(&s, &list),
        ["b", "c", "d"],
        "range from the anchor"
    );
    s.click(&list, 0, ClickMode::Extend);
    assert_eq!(
        selected(&s, &list),
        ["a", "b"],
        "the anchor stays where the range began"
    );
    s.click(&list, 4, ClickMode::Toggle);
    assert_eq!(selected(&s, &list), ["a", "b", "e"]);
    s.click(&list, 0, ClickMode::Toggle);
    assert_eq!(selected(&s, &list), ["b", "e"]);
    assert_eq!(
        s.cursor(),
        Some(&key("a")),
        "the cursor follows the last click"
    );
    s.click(&list, 2, ClickMode::Replace);
    assert_eq!(selected(&s, &list), ["c"]);
}

#[test]
fn select_all_and_clear() {
    let list = rows(&["a", "b", "c"]);
    let mut s = Selection::default();
    s.select_all(&list);
    assert_eq!(s.len(), 3);
    s.clear();
    assert!(s.is_empty());
    assert_eq!(s.cursor(), None);
}

#[test]
fn keys_move_the_cursor_and_clamp_at_the_ends() {
    let list = rows(&["a", "b", "c"]);
    let mut s = Selection::default();
    assert_eq!(
        s.move_cursor(&list, 1, false),
        Some(0),
        "no cursor: down starts at the top"
    );
    assert_eq!(s.move_cursor(&list, 1, false), Some(1));
    assert_eq!(s.move_cursor(&list, 5, false), Some(2), "clamped");
    assert_eq!(selected(&s, &list), ["c"]);
    assert_eq!(s.move_cursor(&list, -1, true), Some(1));
    assert_eq!(
        selected(&s, &list),
        ["b", "c"],
        "shift extends from the anchor"
    );
    let mut fresh = Selection::default();
    assert_eq!(
        fresh.move_cursor(&list, -1, false),
        Some(2),
        "up starts at the bottom"
    );
    assert_eq!(Selection::default().move_cursor(&[], 1, false), None);
}

#[test]
fn a_right_click_outside_the_selection_replaces_it_inside_keeps_it() {
    let list = rows(&["a", "b", "c"]);
    let mut s = Selection::default();
    s.click(&list, 0, ClickMode::Replace);
    s.click(&list, 1, ClickMode::Toggle);
    s.context_click(&list, 1);
    assert_eq!(selected(&s, &list), ["a", "b"]);
    s.context_click(&list, 2);
    assert_eq!(selected(&s, &list), ["c"]);
}

#[test]
fn deltas_keep_the_selection_by_identity_not_index() {
    let mut list = rows(&["a", "b", "c"]);
    let mut s = Selection::default();
    s.click(&list, 1, ClickMode::Replace); // b
    s.click(&list, 2, ClickMode::Toggle); // c
    // A pod lands above b, c is deleted, b is modified in place.
    let ops = vec![
        RowOp::Remove { index: 2 },
        RowOp::Insert {
            index: 0,
            object: Arc::new(StoreObject::Resource(p("x", "0new", "1"))),
        },
        RowOp::Update {
            index: 2,
            object: Arc::new(StoreObject::Resource(p("x", "b", "2"))),
        },
    ];
    s.apply_ops(&mut list, &ops);
    assert_eq!(
        selected(&s, &list),
        ["b"],
        "c went, b stayed although it moved to row 2"
    );
    assert_eq!(s.cursor(), None, "the cursor was on the deleted row");
    assert_eq!(s.cursor_index(&list), None);

    // A row that moves (removed and inserted in one batch) stays selected.
    s.click(&list, 2, ClickMode::Replace); // b
    let moved = vec![
        RowOp::Remove { index: 2 },
        RowOp::Insert {
            index: 0,
            object: Arc::new(StoreObject::Resource(p("x", "b", "3"))),
        },
    ];
    s.apply_ops(&mut list, &moved);
    assert_eq!(selected(&s, &list), ["b"]);
    assert_eq!(s.cursor_index(&list), Some(0));
}

#[test]
fn a_snapshot_keeps_only_listed_selected_rows() {
    let mut list = rows(&["a", "b", "c"]);
    let mut s = Selection::default();
    s.select_all(&list);
    s.click(&list, 0, ClickMode::Toggle); // cursor a, a deselected
    let snapshot = rows(&["c", "d", "a"]);
    s.apply_snapshot(&mut list, &snapshot);
    assert_eq!(list.len(), 3);
    assert_eq!(selected(&s, &list), ["c"]);
    assert_eq!(
        s.cursor_index(&list),
        Some(2),
        "the cursor row is still listed"
    );
}
