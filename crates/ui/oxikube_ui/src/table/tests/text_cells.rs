//! The plain-text cell fast path (E07-S09): a delegate's `TextCell`s are drawn by the table, in
//! their colour and alignment, and only a text too wide for its column takes the ellipsis box.

use crate::table::text_cell::{FIT_SLACK, fits};
use crate::table::{Table, TableColumn, TableDelegate, TableHandle, TextCell};
use gpui::{
    App, Context, IntoElement, ParentElement as _, Render, Styled as _, TestAppContext, Window,
    div, hsla, px,
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

#[gpui::test]
fn text_cells_are_drawn_by_the_table_aligned_and_kept_inside_their_column(cx: &mut TestAppContext) {
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
    let (text_cells, elements) =
        view.read_with(cx, |v, cx| v.table.read(cx, |d| (d.text_cells, d.elements)));
    assert!(text_cells >= 4, "every visible cell came from text_cell");
    assert_eq!(elements, 0, "render_td is the fallback only");

    for row in 0..2 {
        let cell = cx
            .debug_bounds(Box::leak(format!("td-{row}-0").into_boxed_str()))
            .expect("the cell was laid out");
        let name = cx
            .debug_bounds(Box::leak(format!("td-{row}-1").into_boxed_str()))
            .expect("the cell was laid out");
        assert!(
            cell.right() <= name.left() + px(1.),
            "row {row}: the narrow column's text stays inside it ({cell:?} vs {name:?})"
        );
    }
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
