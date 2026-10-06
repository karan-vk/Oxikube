//! [`Column`]: one column of a resource table, and its stable [`ColumnId`].

use std::fmt;
use std::sync::Arc;

/// The stable identity of a column within a kind, for example `ready` or `restarts`.
///
/// It is the key under which column visibility and order are persisted (E07-S03), so an id never
/// changes once released; a title may. Core ids are written by hand in the catalogue; Table
/// columns derive theirs from the server's column name (`Created At` is `created-at`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ColumnId(Arc<str>);

impl ColumnId {
    /// The `metadata.name` column.
    pub const NAME: &'static str = "name";
    /// The `metadata.namespace` column.
    pub const NAMESPACE: &'static str = "namespace";
    /// The age column (time since `metadata.creationTimestamp`).
    pub const AGE: &'static str = "age";
    /// The labels column.
    pub const LABELS: &'static str = "labels";

    /// An id from its text.
    pub fn new(id: impl Into<Arc<str>>) -> Self {
        Self(id.into())
    }

    /// The id as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ColumnId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for ColumnId {
    fn from(id: &str) -> Self {
        Self::new(id)
    }
}

impl PartialEq<str> for ColumnId {
    fn eq(&self, other: &str) -> bool {
        &*self.0 == other
    }
}

impl PartialEq<&str> for ColumnId {
    fn eq(&self, other: &&str) -> bool {
        &*self.0 == *other
    }
}

/// Which side of the cell the text sits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Align {
    /// Text and identifiers. The default.
    #[default]
    Left,
    /// Counts, sizes and ages.
    Right,
}

/// The kind of value a column sorts by, so a view can show the right sort affordance and the
/// store can pick a comparator without looking at a cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SortKind {
    /// Display text, case-insensitively. The default.
    #[default]
    Text,
    /// Integers and real numbers.
    Number,
    /// Resource quantities (`500Mi`, `2Gi`, `250m`).
    Quantity,
    /// Time since something happened.
    Age,
    /// Points in time.
    Time,
}

/// One column of a resource table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    /// The persisted key.
    pub id: ColumnId,
    /// The header text.
    pub title: Arc<str>,
    /// A longer explanation for a header tooltip, when the source has one (Table columns).
    pub description: Option<Arc<str>>,
    /// Hidden by default and offered in the column picker (`kubectl -o wide`). The provider only
    /// flags it; the table decides what is visible.
    pub wide: bool,
    /// Which side the text sits on.
    pub align: Align,
    /// What the column sorts by.
    pub sort: SortKind,
    /// For a Table-feed column, the index of its cell in each row: the `SortField::Column` index
    /// the store sorts by. `None` for core columns, which the store ranks by their cells
    /// ([`SortField::Cell`](crate::store::SortField::Cell)).
    pub table_index: Option<usize>,
}

impl Column {
    /// Whether the column is in the default view (not [`wide`](Self::wide)).
    pub fn is_default(&self) -> bool {
        !self.wide
    }
}
