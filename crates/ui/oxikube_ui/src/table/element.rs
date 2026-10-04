//! The renderable table element.

use super::delegate::TableDelegate;
use super::handle::TableHandle;
use gpui::{App, IntoElement, RenderOnce, Window};
use gpui_component::Sizable as _;
use gpui_component::table::DataTable;

use crate::size::ControlSize;

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
}

impl<D: TableDelegate> Table<D> {
    /// A table over `handle`, bordered, not striped, medium density.
    pub fn new(handle: &TableHandle<D>) -> Self {
        Self {
            handle: handle.clone(),
            stripe: false,
            bordered: true,
            size: ControlSize::Medium,
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

    /// Row density (sets the uniform row height).
    pub fn size(mut self, size: ControlSize) -> Self {
        self.size = size;
        self
    }
}

impl<D: TableDelegate> RenderOnce for Table<D> {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        DataTable::new(self.handle.state())
            .stripe(self.stripe)
            .bordered(self.bordered)
            .with_size(self.size)
    }
}
