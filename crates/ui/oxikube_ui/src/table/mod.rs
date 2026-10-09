//! A virtualised table over our own [`TableDelegate`] trait.
//!
//! Module map:
//! - `delegate`: [`TableDelegate`], the data source. No gpui-component types in its signature.
//! - `column`: [`TableColumn`], [`SortDirection`], [`ColumnAlign`].
//! - `handle`: [`TableHandle`] (retained state, selection, scroll) and [`TableOptions`].
//! - `events`: [`TableEvent`] and [`RowClick`], what the table tells its owner.
//! - `element`: [`Table`], the element views place in their tree.
//! - `text_cell`: [`TextCell`], the fast path for plain text cells (no extra element, the
//!   ellipsis only where the text does not fit).
//! - `line` (private): `LineCell`, the one-leaf element of a text cell that fits its column.
//! - `overflow` (private): the fade the table draws where columns continue past the view.
//! - `widths` (private): column widths under UI zoom (design-time widths, scaled on read).
//! - `adapter` (private): forwards our trait to gpui-component's `TableDelegate`.
//!
//! The table is built on `uniform_list`: only the rows in the viewport are rendered, and row
//! height is uniform (see [`ControlSize`](crate::ControlSize) for density).

mod adapter;
mod column;
mod delegate;
mod element;
mod events;
mod handle;
mod line;
mod overflow;
mod text_cell;
mod widths;

pub use column::{ColumnAlign, SortDirection, TableColumn};
pub use delegate::TableDelegate;
pub use element::Table;
pub use events::{RowClick, TableEvent};
pub use handle::{TableHandle, TableOptions};
pub use text_cell::TextCell;

#[cfg(test)]
mod tests;
