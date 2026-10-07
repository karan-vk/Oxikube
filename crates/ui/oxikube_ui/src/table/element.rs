//! The renderable table element.

use super::delegate::TableDelegate;
use super::handle::TableHandle;
use gpui::{
    App, IntoElement, ParentElement as _, RenderOnce, Styled as _, Window, div,
    prelude::FluentBuilder as _,
};
use gpui_component::Sizable as _;
use gpui_component::table::DataTable;

use super::overflow;
use crate::ActiveTokens as _;
use crate::size::{ControlSize, u};

/// Renders a [`TableHandle`]: a header row and virtualised rows.
///
/// ```ignore
/// Table::new(&self.table).stripe(true)
/// ```
#[derive(IntoElement)]
pub struct Table<D: TableDelegate> {
    handle: TableHandle<D>,
    stripe: bool,
    bordered: bool,
    size: ControlSize,
    cue: bool,
}

impl<D: TableDelegate> Table<D> {
    /// A table over `handle`, bordered, not striped, medium density.
    pub fn new(handle: &TableHandle<D>) -> Self {
        Self {
            handle: handle.clone(),
            stripe: false,
            bordered: true,
            size: ControlSize::Medium,
            cue: true,
        }
    }

    /// Alternate row backgrounds.
    pub fn stripe(mut self, stripe: bool) -> Self {
        self.stripe = stripe;
        self
    }

    /// Draw the outer border.
    pub fn bordered(mut self, bordered: bool) -> Self {
        self.bordered = bordered;
        self
    }

    /// Whether to fade the edge where columns continue past the view (on by default).
    pub fn overflow_cue(mut self, cue: bool) -> Self {
        self.cue = cue;
        self
    }

    /// Row density: sets the uniform row height (and the header height, which is one row).
    ///
    /// The density's design-time height (32 px for medium) is multiplied by the UI zoom on every
    /// render, so rows follow the zoom like the columns and the text do. A [`ControlSize::Size`]
    /// is a design-time row height in pixels. The library styles a pixel height with its medium
    /// cell padding, so the density's own padding and text step are not applied.
    pub fn size(mut self, size: ControlSize) -> Self {
        self.size = size;
        self
    }
}

impl<D: TableDelegate> RenderOnce for Table<D> {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        // The library caches column widths; re-read them (keeping what the user resized) when the
        // UI zoom changed since.
        self.handle.rescale_if_stale(cx);
        let table = DataTable::new(self.handle.state())
            .stripe(self.stripe)
            .bordered(self.bordered)
            // The library reads the row height from this on every render, so a zoom change needs
            // no refresh: hand it the zoomed height rather than the density enum (a fixed 32 px).
            .with_size(ControlSize::Size(u(self.size.table_row_height())));
        // Where columns continue past the view, a fade says so (a platform scrollbar hides until
        // it is used). Painted over the table, reading the scroll position as it paints.
        let cue = self.cue.then(|| {
            overflow::cue(
                self.handle.horizontal_scroll_handle(cx),
                cx.colors().background,
            )
        });
        div()
            .relative()
            .size_full()
            .child(table)
            .when_some(cue, |d, cue| d.child(cue))
    }
}
