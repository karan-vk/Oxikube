//! [`ColumnProvider`]: the one way a table asks what columns a kind has and what is in a cell.

use std::sync::Arc;

use jiff::Timestamp;
use oxikube_domain::Capabilities;
use oxikube_domain::ids::Gvk;

use super::{Cell, Column, ColumnId};
use crate::store::StoreObject;

/// Produces the columns of a kind and the cell of an object in a column (ADR 0006: one trait, two
/// implementations).
///
/// * [`CoreColumns`](super::CoreColumns) serves the core kinds from reflector and metadata feeds
///   with our own definitions: computed columns such as `Ready`, `Status` and `Restarts`.
/// * [`TableColumns`](super::TableColumns) serves a Table feed (CRDs and unknown kinds) from the
///   server's own column definitions.
///
/// Both are plain synchronous functions over cached data: no I/O, no locks held across a call,
/// safe on the UI thread. `columns` is memoised and cheap enough to call on every header render;
/// `cell` is called per visible row per frame and borrows from the object where it can.
pub trait ColumnProvider: Send + Sync {
    /// The columns of `kind` for a session with `caps`, in display order: default columns first,
    /// then the [`wide`](Column::wide) ones. Metrics columns appear only when `caps` has
    /// [`Capabilities::METRICS`] and the provider has a metrics source to read them from.
    fn columns(&self, kind: &Gvk, caps: Capabilities) -> Arc<[Column]>;

    /// The cell of `object` in column `column`, as of `now` (ages are relative to it). A column
    /// the provider does not know gives a blank cell.
    fn cell<'a>(&self, object: &'a StoreObject, column: &ColumnId, now: Timestamp) -> Cell<'a>;

    /// A number that changes whenever the order of the rows by `column` may have changed without
    /// any row's object changing (E12-S09). The store ranks a row when its object arrives and
    /// keeps that rank, so a column whose cells read other state (the Pods column counts the
    /// pods on a node) would otherwise stay sorted by what it once showed. The table asks once a
    /// second while it is sorted by `column`, and ranks again when the number moved. `0`, the
    /// default, is a column whose cells depend on the object alone.
    fn rank_epoch(&self, _column: &ColumnId) -> u64 {
        0
    }
}
