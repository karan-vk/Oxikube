//! The plain-text cell fast path (E07-S09): a delegate's `TextCell`s are drawn by the table, and
//! only a text too wide for its column (or any text while a column is being resized) takes the
//! ellipsis box.

use std::time::Duration;

use crate::size::ControlSize;
use crate::table::text_cell::{FIT_SLACK, fits};
use crate::table::{Table, TableColumn, TableDelegate, TableHandle, TextCell};
use gpui::{
    App, Bounds, Context, CursorStyle, Entity, IntoElement, Modifiers, MouseButton,
    ParentElement as _, Pixels, Render, Styled as _, TestAppContext, VisualTestContext, Window,
    div, hsla, point, px,
};

const SHORT: &str = "1/1";
const LONG: &str = "a-pod-name-far-too-long-for-a-sixty-pixel-column";

/// Two columns: a narrow right-aligned one holding `SHORT` and `LONG` rows, and a wide one.
struct Texts {
    text_cells: usize,
    elements: usize,
}

impl TableDelegate for Texts {
    fn columns_count(&self, _: &App) -> usize {
        2
    }
    fn rows_count(&self, _: &App) -> usize {
        2
    }
    fn column(&self, col_ix: usize, _: &App) -> TableColumn {
        match col_ix {
            0 => TableColumn::new("ready", "Ready").width(px(60.)).right(),
            _ => TableColumn::new("name", "Name").width(px(400.)),
        }
    }
    fn text_cell(
        &mut self,
        row_ix: usize,
        _: usize,
        _: &mut Window,
        _: &mut App,
    ) -> Option<TextCell> {
        self.text_cells += 1;
        let text = if row_ix == 0 { SHORT } else { LONG };
        Some(TextCell::new(text).color(hsla(0.3, 0.5, 0.5, 1.)))
    }
    fn render_td(&mut self, _: usize, _: usize, _: &mut Window, _: &mut App) -> impl IntoElement {
        self.elements += 1;
        div()
    }
}

struct View {
    table: TableHandle<Texts>,
}

impl Render for View {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(Table::new(&self.table))
    }
}

fn open(cx: &mut TestAppContext) -> (Entity<View>, &mut VisualTestContext) {
    cx.update(crate::init);
    let (view, cx) = cx.add_window_view(|window, cx| {
        let delegate = Texts {
            text_cells: 0,
            elements: 0,
        };
        View {
            table: TableHandle::new(delegate, window, cx),
        }
    });
    cx.run_until_parked();
    (view, cx)
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

/// Whether the cell at (`row`, `col`) was last drawn through the ellipsis box.
fn has_ellipsis(cx: &mut VisualTestContext, row: usize, col: usize) -> bool {
    let selector = Box::leak(format!("td-ellipsis-{row}-{col}").into_boxed_str());
    cx.debug_bounds(selector).is_some()
}

fn cell_bounds(cx: &mut VisualTestContext, row: usize, col: usize) -> Bounds<Pixels> {
    let selector = Box::leak(format!("td-{row}-{col}").into_boxed_str());
    cx.debug_bounds(selector).expect("the cell was laid out")
}

#[gpui::test]
fn only_a_text_too_wide_for_its_column_takes_the_ellipsis_box(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    let (text_cells, elements) =
        view.read_with(cx, |v, cx| v.table.read(cx, |d| (d.text_cells, d.elements)));
    assert!(text_cells >= 4, "every visible cell came from text_cell");
    assert_eq!(elements, 0, "render_td is the fallback only");

    // The branch each cell took is the one `fits` gives for its column width (60 and 400 px at
    // 100 % zoom): the short text is a bare text, the long one is truncated in the 60 px column.
    for (row, text) in [(0, SHORT), (1, LONG)] {
        for (col, width) in [(0, px(60.)), (1, px(400.))] {
            let expected = !cx.update(|window, _| fits(&TextCell::new(text), width, window));
            assert_eq!(
                has_ellipsis(cx, row, col),
                expected,
                "cell ({row}, {col}) holding {text:?}: ellipsis box expected {expected}"
            );
        }
    }
    assert!(!has_ellipsis(cx, 0, 0), "a text that fits is drawn bare");
    assert!(
        has_ellipsis(cx, 1, 0),
        "a text too wide gets the ellipsis box"
    );

    // The ellipsis box is as wide as its cell, so the truncated text ends inside the column.
    let ellipsis = cx
        .debug_bounds("td-ellipsis-1-0")
        .expect("the ellipsis box was laid out");
    let cell = cell_bounds(cx, 1, 0);
    assert!(
        ellipsis.right() <= cell.right() + px(0.5),
        "the truncated text overflows its column ({ellipsis:?} in {cell:?})"
    );
}

#[gpui::test]
fn a_column_being_resized_draws_its_texts_with_the_ellipsis(cx: &mut TestAppContext) {
    let (_view, cx) = open(cx);
    draw(cx);
    assert!(!has_ellipsis(cx, 0, 0), "\"1/1\" fits the 60 px column");

    // Column 0's resize handle is the right edge of the column, in the header above row 0.
    let padding = ControlSize::Size(px(0.)).table_cell_padding();
    let cell = cell_bounds(cx, 0, 0);
    let column_right = cell.right() + padding.right;
    let header = point(column_right - px(1.), cell.top() - padding.top - px(6.));

    // Drag it down to its 20 px minimum; the library reports the width only on mouse-up.
    let none = Modifiers::default();
    cx.simulate_mouse_down(header, MouseButton::Left, none);
    for step in 1..=4 {
        let x = column_right - px(10. * step as f32);
        cx.simulate_mouse_move(point(x, header.y), MouseButton::Left, none);
    }
    draw(cx);
    assert!(
        cx.update(|_, cx| cx.active_drag_cursor_style()) == Some(CursorStyle::ResizeColumn),
        "the test is dragging the column's resize handle"
    );
    assert!(
        has_ellipsis(cx, 0, 0),
        "mid-drag, a text that fit the old width is drawn with the ellipsis"
    );

    // On mouse-up the new width is known: the 20 px column fits nothing, the 400 px one fits
    // "1/1" again.
    cx.simulate_mouse_up(
        point(column_right - px(40.), header.y),
        MouseButton::Left,
        none,
    );
    draw(cx);
    assert!(!cx.update(|_, cx| cx.has_active_drag()), "the drag ended");
    assert!(has_ellipsis(cx, 0, 0), "the narrowed column truncates");
    assert!(
        !has_ellipsis(cx, 0, 1),
        "an untouched column draws fitting text bare"
    );
}

#[gpui::test]
fn a_text_fits_only_with_room_to_spare(cx: &mut TestAppContext) {
    cx.update(crate::init);
    let (_, cx) = cx.add_window_view(|_, _| EmptyView);
    cx.update(|window, _| {
        assert!(fits(&TextCell::new(SHORT), px(60.), window));
        assert!(!fits(&TextCell::new(LONG), px(60.), window));
        assert!(fits(&TextCell::new(LONG), px(2000.), window));
        // Nothing fits a column narrower than its padding.
        assert!(!fits(&TextCell::new(""), px(10.), window));
        assert!(FIT_SLACK > px(0.));
    });
}

struct EmptyView;

impl Render for EmptyView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

#[test]
fn a_text_cell_carries_its_text_and_colour() {
    let red = hsla(0., 1., 0.5, 1.);
    let cell = TextCell::new("Running").color(red);
    assert_eq!(cell.text.as_ref(), "Running");
    assert_eq!(cell.color, Some(red));
    assert_eq!(TextCell::new("x").color, None);
}

#[gpui::test]
fn a_cut_text_shows_its_full_text_in_a_tooltip_and_a_fitting_one_has_none(cx: &mut TestAppContext) {
    let (_view, cx) = open(cx);
    draw(cx);
    assert!(cx.debug_bounds("td-tooltip").is_none(), "nothing hovered");

    // Hover a text that fits: it is all on screen already, so nothing to add.
    let fitting = cell_bounds(cx, 0, 0);
    cx.simulate_mouse_move(fitting.center(), None, Modifiers::default());
    cx.executor().advance_clock(Duration::from_secs(2));
    draw(cx);
    assert!(
        cx.debug_bounds("td-tooltip").is_none(),
        "a cell that fits has no tooltip"
    );

    // Hover the cut one: the tooltip carries the whole text.
    let cut = cell_bounds(cx, 1, 0);
    cx.simulate_mouse_move(cut.center(), None, Modifiers::default());
    cx.executor().advance_clock(Duration::from_secs(2));
    draw(cx);
    cx.executor().advance_clock(Duration::from_secs(2));
    draw(cx);
    assert!(
        cx.debug_bounds("td-tooltip").is_some(),
        "a cut cell shows a tooltip on hover"
    );
}
