//! The horizontal-overflow cue (E07-U561): the table reports where columns continue past the view.

use crate::table::{HorizontalOverflow, Table, TableColumn, TableDelegate, TableHandle};
use gpui::{
    App, Context, Entity, IntoElement, ParentElement as _, Pixels, Render, Styled as _,
    TestAppContext, VisualTestContext, Window, div, point, px,
};

/// `columns` columns, each `width` wide, three rows.
struct Wide {
    columns: usize,
    width: f32,
}

impl TableDelegate for Wide {
    fn columns_count(&self, _: &App) -> usize {
        self.columns
    }
    fn rows_count(&self, _: &App) -> usize {
        3
    }
    fn column(&self, col_ix: usize, _: &App) -> TableColumn {
        TableColumn::new(format!("c{col_ix}"), format!("C{col_ix}")).width(px(self.width))
    }
    fn render_td(&mut self, _: usize, _: usize, _: &mut Window, _: &mut App) -> impl IntoElement {
        div().child("x")
    }
}

struct View {
    table: TableHandle<Wide>,
}

impl Render for View {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(Table::new(&self.table))
    }
}

fn open(
    cx: &mut TestAppContext,
    columns: usize,
    width: f32,
) -> (Entity<View>, &mut VisualTestContext) {
    cx.update(crate::init);
    let (view, cx) = cx.add_window_view(|window, cx| View {
        table: TableHandle::new(Wide { columns, width }, window, cx),
    });
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    (view, cx)
}

fn overflow(view: &Entity<View>, cx: &mut VisualTestContext) -> HorizontalOverflow {
    view.read_with(cx, |v, cx| v.table.horizontal_overflow(cx))
}

#[gpui::test]
fn a_table_that_fits_shows_no_cue(cx: &mut TestAppContext) {
    let (view, cx) = open(cx, 3, 100.);
    assert_eq!(overflow(&view, cx), HorizontalOverflow::default());
}

#[gpui::test]
fn columns_past_the_view_show_the_cue_on_the_side_that_has_more(cx: &mut TestAppContext) {
    let (view, cx) = open(cx, 40, 300.);
    assert_eq!(
        overflow(&view, cx),
        HorizontalOverflow {
            left: false,
            right: true
        },
        "at the start, more columns lie to the right"
    );

    // Scroll to the end: the cue moves to the left edge.
    view.update_in(cx, |v, _, cx| {
        let handle = v.table.horizontal_scroll_handle(cx);
        let max: Pixels = handle.max_offset().x;
        handle.set_offset(point(-max, px(0.)));
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert_eq!(
        overflow(&view, cx),
        HorizontalOverflow {
            left: true,
            right: false
        }
    );
}
