//! Table columns and cells: what a kind's table shows and what is in each cell (E07-S02).
//!
//! The UI asks one question of one trait, [`ColumnProvider`]: "what columns does this kind have
//! and what is in this cell". Two implementations answer it (ADR 0006):
//!
//! | Provider | Serves | Columns from |
//! |---|---|---|
//! | [`CoreColumns`] | core kinds on reflector or metadata feeds (~40 kinds) | our own catalogue: computed `Ready`, `Status`, `Restarts`, ... |
//! | [`TableColumns`] | CRDs and unknown kinds on a Table feed | the server's `columnDefinitions` (`additionalPrinterColumns`) |
//!
//! A [`Column`] has a stable [`ColumnId`], a title, a `wide` flag (hidden by default, as in
//! `kubectl -o wide`), an alignment and a [`SortKind`]. A [`Cell`] has display text, a typed
//! [`CellSort`] key (so sorting never reparses text) and a [`Tone`] (a meaning, never a colour).
//! CPU and memory columns are hooks: [`Cell::Pending`] until E13 registers a [`MetricsSource`].
//!
//! Everything here is pure, synchronous and allocation-light: no gpui, no kube, no I/O.

mod builtin;
mod cell;
mod column;
mod provider;
mod table;

#[cfg(test)]
mod tests;

pub use builtin::{CoreColumns, Metric, MetricsSource};
pub use cell::{Cell, CellSort, CellValue, Tone};
pub use column::{Align, Column, ColumnId, SortKind};
pub use provider::ColumnProvider;
pub use table::TableColumns;
