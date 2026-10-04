//! Table tests: virtualisation, delegate forwarding, events.

mod support;

use self::support::harness;
use super::*;
use crate::size::{UiScale, Unscaled, set_ui_scale};
use gpui::{TestAppContext, VisualTestContext, px};
use gpui_component::table::TableEvent as LibEvent;

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
    let lib = column.to_library(UiScale::IDENTITY, None);
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

#[test]
fn library_columns_scale_design_widths_and_prefer_user_widths() {
    let column = TableColumn::new("k", "K")
        .width(px(200.))
        .min_width(px(40.));
    let zoomed = UiScale::new(1.5);
    let lib = column.to_library(zoomed, None);
    assert_eq!(lib.width, px(300.));
    assert_eq!(lib.min_width, px(60.));
    // A width the user chose is unscaled and replaces the design width; the minimum still holds.
    assert_eq!(
        column.to_library(zoomed, Some(Unscaled(100.))).width,
        px(150.)
    );
    assert_eq!(
        column.to_library(zoomed, Some(Unscaled(10.))).width,
        px(60.)
    );
}

/// Width the table gave column 0's header body (tagged `th-0` by the harness delegate).
fn header_width(cx: &mut VisualTestContext) -> f32 {
    let bounds = cx.debug_bounds("th-0").expect("header was not laid out");
    f32::from(bounds.size.width)
}

fn set_zoom_and_draw(cx: &mut VisualTestContext, factor: f32) {
    cx.update(|window, cx| {
        set_ui_scale(cx, UiScale::new(factor));
        window.refresh();
    });
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

#[gpui::test]
fn column_widths_follow_ui_zoom_after_creation(cx: &mut TestAppContext) {
    let (_view, cx) = harness(cx, 5);
    cx.run_until_parked();
    let at_100 = header_width(cx);

    // 150 design px at 200 % is 300 px: the cached width must have been re-read.
    set_zoom_and_draw(cx, 2.0);
    let at_200 = header_width(cx);
    assert!(
        (at_200 - at_100 - 150.).abs() < 1.5,
        "column 0 header is {at_100} px at 100 % and {at_200} px at 200 %: widths did not follow zoom"
    );

    set_zoom_and_draw(cx, 1.0);
    assert!(
        (header_width(cx) - at_100).abs() < 1.5,
        "widths did not return at 100 %"
    );
}

/// Heights the table gave column 0's header body and row 0's cell body.
fn header_and_row_heights(cx: &mut VisualTestContext) -> (f32, f32) {
    let mut height = |selector: &'static str| {
        let bounds = cx.debug_bounds(selector).expect("cell was not laid out");
        f32::from(bounds.size.height)
    };
    (height("th-0"), height("td-0-0"))
}

#[gpui::test]
fn row_and_header_heights_follow_ui_zoom(cx: &mut TestAppContext) {
    let (_view, cx) = harness(cx, 5);
    cx.run_until_parked();
    let (header_100, row_100) = header_and_row_heights(cx);

    // The medium density is 32 design px, so 200 % adds 32 px to both (cell padding is fixed).
    set_zoom_and_draw(cx, 2.0);
    let (header_200, row_200) = header_and_row_heights(cx);
    assert!(
        (header_200 - header_100 - 32.).abs() < 1.5,
        "header is {header_100} px at 100 % and {header_200} px at 200 %: height ignores zoom"
    );
    assert!(
        (row_200 - row_100 - 32.).abs() < 1.5,
        "row is {row_100} px at 100 % and {row_200} px at 200 %: height ignores zoom"
    );

    set_zoom_and_draw(cx, 1.0);
    let (header_back, row_back) = header_and_row_heights(cx);
    assert!(
        (header_back - header_100).abs() < 1.5 && (row_back - row_100).abs() < 1.5,
        "heights did not return at 100 %"
    );
}

#[gpui::test]
fn user_resized_widths_survive_a_zoom_change_unscaled(cx: &mut TestAppContext) {
    let (view, cx) = harness(cx, 5);
    cx.run_until_parked();
    set_zoom_and_draw(cx, 2.0);
    let at_200 = header_width(cx);

    let table = view.read_with(cx, |h, _| h.table.clone());
    let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let sink = events.clone();
    let _subscription = cx
        .update(|_, cx| table.on_event(cx, move |event, _| sink.borrow_mut().push(event.clone())));

    // The user drags column 0 from 300 to 400 on-screen pixels at 200 % (columns 1 and 2 stay).
    let state = table.state().clone();
    cx.update(|_, cx| {
        state.update(cx, |_, cx| {
            cx.emit(LibEvent::ColumnWidthsChanged(vec![
                px(400.),
                px(300.),
                px(300.),
            ]));
        })
    });
    assert_eq!(
        events.borrow().last(),
        Some(&TableEvent::ColumnsResized(vec![
            Unscaled(200.),
            Unscaled(150.),
            Unscaled(150.)
        ])),
        "resize events carry unscaled widths"
    );

    // At 100 % the dragged column is its 200 design px, not the delegate's 150.
    set_zoom_and_draw(cx, 1.0);
    let at_100 = header_width(cx);
    assert!(
        (at_200 - at_100 - 100.).abs() < 1.5,
        "user width lost on zoom: {at_200} px at 200 % before the drag, {at_100} px at 100 % after"
    );

    // `refresh` is the explicit reset: the delegate's width wins again.
    cx.update(|_, cx| table.refresh(cx));
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(
        (header_width(cx) - at_100 + 50.).abs() < 1.5,
        "refresh kept the user width"
    );
}

#[gpui::test]
fn dragging_a_column_back_to_its_supplied_width_replaces_the_override(cx: &mut TestAppContext) {
    let (view, cx) = harness(cx, 5);
    cx.run_until_parked();
    let baseline = header_width(cx);

    let table = view.read_with(cx, |h, _| h.table.clone());
    let state = table.state().clone();
    let resize = |cx: &mut VisualTestContext, first: f32| {
        cx.update(|_, cx| {
            state.update(cx, |_, cx| {
                cx.emit(LibEvent::ColumnWidthsChanged(vec![
                    px(first),
                    px(150.),
                    px(150.),
                ]));
            })
        });
    };

    // Column 0 is 150 px at 100 %. The user drags it to 200, then back to 150.
    resize(cx, 200.);
    resize(cx, 150.);

    // The user left it at 150, so that is what zoom must keep: not the stale 200.
    set_zoom_and_draw(cx, 2.0);
    let at_200 = header_width(cx);
    assert!(
        (at_200 - baseline - 150.).abs() < 1.5,
        "header is {baseline} px at 100 % and {at_200} px at 200 %: expected 150 design px more"
    );
    set_zoom_and_draw(cx, 1.0);
    assert!(
        (header_width(cx) - baseline).abs() < 1.5,
        "column 0 header is {} px back at 100 %, expected {baseline}",
        header_width(cx)
    );
}
