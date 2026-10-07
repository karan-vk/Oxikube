//! The table-driven definition of a core column: [`ColumnDef`] and where its value comes from
//! ([`Src`]).
//!
//! A catalogue entry is a `const`, so the whole registry lives in the binary's read-only data and
//! building a [`Column`](crate::columns::Column) from it is the only allocation. Simple columns
//! name a JSON pointer, which is resolved without allocating; computed ones name a function.

use jiff::Timestamp;
use oxikube_domain::Resource;

use crate::columns::{Align, Cell, SortKind};

/// A computed cell: reads whatever it needs from the resource and borrows from it where it can.
pub(crate) type CellFn = for<'a> fn(&'a Resource, Timestamp) -> Cell<'a>;

/// The metrics a column can be a hook for (E13 fills the values).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Metric {
    /// CPU usage.
    Cpu,
    /// Memory usage.
    Memory,
}

/// Where a core column's cell comes from.
#[derive(Clone, Copy)]
pub(crate) enum Src {
    /// `metadata.name`.
    Name,
    /// `metadata.namespace`.
    Namespace,
    /// Time since `metadata.creationTimestamp`.
    Age,
    /// `metadata.labels` as `k=v,k=v`.
    Labels,
    /// The string at a JSON pointer (numbers and booleans print as text); blank when absent.
    Text(&'static str),
    /// The integer at a JSON pointer, sorted numerically; blank when absent.
    Int(&'static str),
    /// The quantity string at a JSON pointer, shown as written and sorted by value.
    Qty(&'static str),
    /// The number of entries of the array or object at a JSON pointer (`0` when absent).
    Len(&'static str),
    /// A computed cell.
    Func(CellFn),
    /// A metrics hook: [`Cell::Pending`] until a metrics source supplies a value.
    Metric(Metric),
}

/// One core column: identity, title, flags and source.
#[derive(Clone, Copy)]
pub(crate) struct ColumnDef {
    pub id: &'static str,
    pub title: &'static str,
    pub src: Src,
    pub wide: bool,
    pub align: Align,
    pub sort: SortKind,
}

impl ColumnDef {
    /// A left-aligned, default-visible column sorted as text.
    pub const fn new(id: &'static str, title: &'static str, src: Src) -> Self {
        Self {
            id,
            title,
            src,
            wide: false,
            align: Align::Left,
            sort: SortKind::Text,
        }
    }

    /// Hidden by default (`kubectl -o wide`).
    pub const fn wide(mut self) -> Self {
        self.wide = true;
        self
    }

    /// Right-aligned, sorted as a number.
    pub const fn number(mut self) -> Self {
        self.align = Align::Right;
        self.sort = SortKind::Number;
        self
    }

    /// Sorted by quantity, right-aligned.
    pub const fn quantity(mut self) -> Self {
        self.align = Align::Right;
        self.sort = SortKind::Quantity;
        self
    }

    /// Sorted by age, right-aligned.
    pub const fn age(mut self) -> Self {
        self.align = Align::Right;
        self.sort = SortKind::Age;
        self
    }

    /// Whether this is a metrics hook.
    pub const fn is_metric(&self) -> bool {
        matches!(self.src, Src::Metric(_))
    }
}

/// One kind: its API group and kind name, and its columns in display order (default columns
/// first, then wide ones).
pub(crate) struct KindDef {
    pub group: &'static str,
    pub kind: &'static str,
    pub columns: &'static [ColumnDef],
}

/// The columns every kind starts from.
pub(crate) const NAME: ColumnDef = ColumnDef::new("name", "Name", Src::Name);
/// `metadata.namespace`.
pub(crate) const NAMESPACE: ColumnDef = ColumnDef::new("namespace", "Namespace", Src::Namespace);
/// Time since creation.
pub(crate) const AGE: ColumnDef = ColumnDef::new("age", "Age", Src::Age).age();
/// Labels, hidden by default.
pub(crate) const LABELS: ColumnDef = ColumnDef::new("labels", "Labels", Src::Labels).wide();

/// What an unknown kind gets: the columns every object has.
pub(crate) const GENERIC: &[ColumnDef] = &[NAME, NAMESPACE, AGE, LABELS];

/// CPU and memory usage hooks (offered only with the `METRICS` capability and a registered
/// [`MetricsSource`](super::MetricsSource); E13 supplies values).
pub(crate) const CPU: ColumnDef = ColumnDef::new("cpu", "CPU", Src::Metric(Metric::Cpu)).quantity();
/// See [`CPU`].
pub(crate) const MEMORY: ColumnDef =
    ColumnDef::new("memory", "Memory", Src::Metric(Metric::Memory)).quantity();
