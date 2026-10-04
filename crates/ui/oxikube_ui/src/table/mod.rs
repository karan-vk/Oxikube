//! A virtualised table over our own [`TableDelegate`] trait.
//!
//! Module map:
//! - `delegate`: [`TableDelegate`], the data source. No gpui-component types in its signature.
//! - `column`: [`TableColumn`], [`SortDirection`], [`ColumnAlign`].
//! - `handle`: [`TableHandle`] (retained state, selection, scroll) and [`TableEvent`].
//! - `element`: [`Table`], the element views place in their tree.
//! - `widths` (private): column widths under UI zoom (design-time widths, scaled on read).
//! - `adapter` (private): forwards our trait to gpui-component's `TableDelegate`.
//!
//! The table is built on `uniform_list`: only the rows in the viewport are rendered, and row
//! height is uniform (see [`ControlSize`](crate::ControlSize) for density).

mod adapter;
mod column;
mod delegate;
mod element;
mod handle;
mod widths;

pub use column::{ColumnAlign, SortDirection, TableColumn};
pub use delegate::TableDelegate;
pub use element::Table;
pub use handle::{TableEvent, TableHandle, TableOptions};

#[cfg(test)]
mod tests;
