//! Table tests: virtualisation, delegate forwarding, events.

mod support;

use self::support::harness;
use super::*;
use gpui::TestAppContext;

const ROWS: usize = 10_000;

#[gpui::test]
fn ten_thousand_rows_build_only_the_visible_ones(cx: &mut TestAppContext) {
    let (view, cx) = harness(cx, ROWS);
    cx.run_until_parked();

    let counts = view.read_with(cx, |h, _| h.counts.clone());
    let rendered = counts.cells_rendered();
    assert!(rendered > 0, "nothing rendered: the table never drew");
    // A ~768 px window shows a few dozen rows of 3 columns; 10k rows would be 30k cells.
    assert!(
        rendered < 300,
        "rendered {rendered} cells for {ROWS} rows: the table is not virtualised"
    );
    assert!(
        counts.max_row_rendered() < 100,
        "rendered rows far below the fold"
    );

    let visible = view.read_with(cx, |h, cx| h.table.visible_rows(cx));
    assert_eq!(visible.start, 0);
    assert!(
        visible.end > 5 && visible.end < 200,
        "visible = {visible:?}"
    );
}

#[gpui::test]
fn scrolling_renders_the_new_rows_not_all_rows(cx: &mut TestAppContext) {
    let (view, cx) = harness(cx, ROWS);
    cx.run_until_parked();
    view.update_in(cx, |h, _, cx| h.table.scroll_to_row(5_000, cx));
    cx.run_until_parked();

    let counts = view.read_with(cx, |h, _| h.counts.clone());
    assert!(
        counts.max_row_rendered() >= 5_000,
        "scroll did not reach row 5000"
    );
    assert!(
        counts.distinct_rows_rendered() < 400,
        "scrolling rendered too many rows"
    );
}

#[gpui::test]
fn update_changes_rows_without_losing_the_table(cx: &mut TestAppContext) {
    let (view, cx) = harness(cx, 10);
    cx.run_until_parked();
    view.update_in(cx, |h, _, cx| h.table.update(cx, |d| d.rows = 3));
    cx.run_until_parked();
    let rows = view.read_with(cx, |h, cx| h.table.read(cx, |d| d.rows));
    assert_eq!(rows, 3);
    let visible = view.read_with(cx, |h, cx| h.table.visible_rows(cx));
    assert!(visible.end <= 3, "visible range {visible:?} exceeds 3 rows");
}

#[gpui::test]
fn selection_round_trips_and_emits_events(cx: &mut TestAppContext) {
    let (view, cx) = harness(cx, 50);
    cx.run_until_parked();
    let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let sink = events.clone();
    let table = view.read_with(cx, |h, _| h.table.clone());
    let _subscription = cx
        .update(|_, cx| table.on_event(cx, move |event, _| sink.borrow_mut().push(event.clone())));

    cx.update(|_, cx| table.select_row(7, cx));
    assert_eq!(cx.update(|_, cx| table.selected_row(cx)), Some(7));
    assert_eq!(events.borrow().first(), Some(&TableEvent::SelectRow(7)));

    cx.update(|_, cx| table.clear_selection(cx));
    assert_eq!(cx.update(|_, cx| table.selected_row(cx)), None);
    assert_eq!(events.borrow().last(), Some(&TableEvent::SelectionCleared));
}

#[test]
fn column_builders_map_to_the_library_column() {
    use gpui::{TextAlign, px};
    let column = TableColumn::new("age", "Age")
        .width(px(80.))
        .min_width(px(60.))
        .right()
        .sorted(SortDirection::Descending)
        .movable(false)
        .fixed_left();
    let lib = column.to_library();
    assert_eq!(lib.key.as_ref(), "age");
    assert_eq!(lib.name.as_ref(), "Age");
    assert_eq!(lib.width, px(80.));
    assert_eq!(lib.min_width, px(60.));
    assert_eq!(lib.align, TextAlign::Right);
    assert_eq!(
        lib.sort,
        Some(gpui_component::table::ColumnSort::Descending)
    );
    assert!(!lib.movable);
    assert!(lib.fixed.is_some());
}

#[test]
fn width_never_drops_below_min_width() {
    use gpui::px;
    let column = TableColumn::new("k", "K").min_width(px(50.)).width(px(10.));
    assert_eq!(column.width, px(50.));
}

#[test]
fn sort_direction_round_trips() {
    use gpui_component::table::ColumnSort;
    for sort in [
        SortDirection::Unsorted,
        SortDirection::Ascending,
        SortDirection::Descending,
    ] {
        assert_eq!(SortDirection::from(ColumnSort::from(sort)), sort);
    }
}
