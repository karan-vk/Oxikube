//! [`SortKey`]: the order a subscriber's rows are kept in, and the [`SortValue`] each object is
//! ranked by.
//!
//! Besides the metadata fields, a subscriber can sort by any table column through
//! [`SortField::Cell`]: the object ranks by the typed sort key of its cell in that column
//! ([`CellSort`]), read once per object version from the view's
//! [`ColumnProvider`], so `9` sorts before `10`, `500Mi` before `2Gi` and `3m` before `2d`,
//! exactly as [`Cell::compare`](crate::columns::Cell::compare) orders two cells.

use std::cmp::Ordering;
use std::fmt;
use std::sync::Arc;

use jiff::Timestamp;
use oxikube_domain::{Age, Quantity};
use serde_json::Value;

use super::object::StoreObject;
use crate::columns::{Cell, CellSort, ColumnId, ColumnProvider};

/// What to sort by.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum SortField {
    /// Namespace, then name (kubectl's order). The default.
    #[default]
    Namespace,
    /// Name, then namespace.
    Name,
    /// `metadata.creationTimestamp` (oldest first when ascending).
    Created,
    /// The value of one label (objects without it last).
    Label(String),
    /// One Table cell, by column index (objects without cells last).
    Column(usize),
    /// One table column of a [`ColumnProvider`], by its cells' typed sort keys (blank and
    /// pending cells last). The column the user clicked in a resource table.
    Cell(CellSortKey),
}

/// A column of a [`ColumnProvider`] used as a sort field ([`SortField::Cell`]).
///
/// Ages are read against a fixed far-future instant rather than the clock, so an object's rank
/// never drifts while it sits in the index: a longer age still means an older object.
#[derive(Clone)]
pub struct CellSortKey {
    /// The column.
    pub column: ColumnId,
    /// Who reads the cells (the table's provider).
    pub provider: Arc<dyn ColumnProvider>,
}

impl CellSortKey {
    /// Sort by `column` of `provider`.
    pub fn new(column: ColumnId, provider: Arc<dyn ColumnProvider>) -> Self {
        Self { column, provider }
    }

    fn value_of(&self, object: &StoreObject) -> SortValue {
        SortValue::of_cell(&self.provider.cell(object, &self.column, Timestamp::MAX))
    }
}

impl PartialEq for CellSortKey {
    /// Same column of the same provider instance.
    fn eq(&self, other: &Self) -> bool {
        self.column == other.column
            && std::ptr::addr_eq(Arc::as_ptr(&self.provider), Arc::as_ptr(&other.provider))
    }
}

impl Eq for CellSortKey {}

impl fmt::Debug for CellSortKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("CellSortKey").field(&self.column).finish()
    }
}

/// A sort field and direction.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SortKey {
    /// The field.
    pub field: SortField,
    /// Reverse the order.
    pub descending: bool,
}

impl SortKey {
    /// Ascending by `field`.
    pub fn by(field: SortField) -> Self {
        Self {
            field,
            descending: false,
        }
    }

    /// The same field, descending.
    #[must_use]
    pub fn descending(mut self) -> Self {
        self.descending = true;
        self
    }

    /// The value `object` is ranked by under this key. Ties break on the object key.
    pub(crate) fn value_of(&self, object: &StoreObject) -> SortValue {
        let meta = object.meta();
        match &self.field {
            SortField::Namespace => SortValue::Missing,
            SortField::Name => SortValue::Text(meta.name.clone()),
            SortField::Created => meta.creation.map_or(SortValue::Missing, SortValue::Time),
            SortField::Label(key) => meta
                .labels
                .get(key.as_str())
                .map_or(SortValue::Missing, |v| SortValue::Text(v.clone())),
            SortField::Column(i) => object
                .cells()
                .and_then(|cells| cells.get(*i))
                .map_or(SortValue::Missing, cell_value),
            SortField::Cell(key) => key.value_of(object),
        }
    }
}

fn cell_value(cell: &Value) -> SortValue {
    match cell {
        Value::Number(n) => n
            .as_i64()
            .map(SortValue::Int)
            .or_else(|| n.as_f64().map(SortValue::Float))
            .unwrap_or(SortValue::Missing),
        Value::String(s) => SortValue::Text(Arc::from(s.as_str())),
        Value::Bool(b) => SortValue::Int(i64::from(*b)),
        Value::Null | Value::Array(_) | Value::Object(_) => SortValue::Missing,
    }
}

/// The value an object is ranked by. Numbers sort before quantities, quantities before ages,
/// ages before text, text before times, and a missing value after everything (so blanks sink to
/// the bottom when ascending): the same ladder as [`Cell::compare`].
#[derive(Debug, Clone)]
pub(crate) enum SortValue {
    Int(i64),
    Float(f64),
    Quantity(Quantity),
    Age(Age),
    Text(Arc<str>),
    Time(Timestamp),
    Missing,
}

impl SortValue {
    /// A cell's typed sort key as a rank value. Text is folded to ASCII lower case, as
    /// [`Cell::compare`] compares it.
    fn of_cell(cell: &Cell<'_>) -> Self {
        match cell.sort() {
            CellSort::None => SortValue::Missing,
            CellSort::Int(n) => SortValue::Int(n),
            CellSort::Float(n) => SortValue::Float(n),
            CellSort::Quantity(q) => SortValue::Quantity(q),
            CellSort::Age(a) => SortValue::Age(a),
            CellSort::Time(t) => SortValue::Time(t),
            CellSort::Text => SortValue::Text(Arc::from(cell.display().to_ascii_lowercase())),
        }
    }

    fn rank(&self) -> u8 {
        match self {
            SortValue::Int(_) | SortValue::Float(_) => 0,
            SortValue::Quantity(_) => 1,
            SortValue::Age(_) => 2,
            SortValue::Text(_) => 3,
            SortValue::Time(_) => 4,
            SortValue::Missing => 5,
        }
    }
}

impl Ord for SortValue {
    fn cmp(&self, other: &Self) -> Ordering {
        use SortValue::{Age, Float, Int, Quantity, Text, Time};
        match (self, other) {
            (Int(a), Int(b)) => a.cmp(b),
            #[allow(
                clippy::cast_precision_loss,
                reason = "mixed int/float cells compare as f64"
            )]
            (Int(a), Float(b)) => (*a as f64).total_cmp(b),
            #[allow(
                clippy::cast_precision_loss,
                reason = "mixed int/float cells compare as f64"
            )]
            (Float(a), Int(b)) => a.total_cmp(&(*b as f64)),
            (Float(a), Float(b)) => a.total_cmp(b),
            (Quantity(a), Quantity(b)) => a.cmp(b),
            (Age(a), Age(b)) => a.cmp(b),
            (Text(a), Text(b)) => a.cmp(b),
            (Time(a), Time(b)) => a.cmp(b),
            _ => self.rank().cmp(&other.rank()),
        }
    }
}

impl PartialOrd for SortValue {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for SortValue {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for SortValue {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::columns::CoreColumns;
    use oxikube_testkit::pod;

    #[test]
    fn sort_values_order_numbers_text_time_then_missing() {
        let mut values = [
            SortValue::Missing,
            SortValue::Text("b".into()),
            SortValue::Float(1.5),
            SortValue::Int(2),
            SortValue::Age(oxikube_domain::Age::from_secs(5)),
            SortValue::Text("a".into()),
            SortValue::Int(1),
        ];
        values.sort();
        assert!(matches!(values[0], SortValue::Int(1)));
        assert!(matches!(values[1], SortValue::Float(_)));
        assert!(matches!(values[2], SortValue::Int(2)));
        assert!(matches!(values[3], SortValue::Age(_)));
        assert!(matches!(&values[4], SortValue::Text(t) if &**t == "a"));
        assert!(matches!(values[6], SortValue::Missing));
    }

    #[test]
    fn cell_keys_rank_by_the_typed_cell_value() {
        let provider: Arc<dyn ColumnProvider> = Arc::new(CoreColumns::new());
        let key = SortKey::by(SortField::Cell(CellSortKey::new(
            ColumnId::new("restarts"),
            provider.clone(),
        )));
        let rank = |n: u32| {
            key.value_of(&StoreObject::Resource(
                pod().name(format!("p{n}")).restarts(n).build(),
            ))
        };
        // As text "10" < "9"; as counts 9 < 10.
        assert!(rank(9) < rank(10));

        let names = SortKey::by(SortField::Cell(CellSortKey::new(
            ColumnId::new(ColumnId::NAME),
            provider.clone(),
        )));
        let name = |n: &str| names.value_of(&StoreObject::Resource(pod().name(n).build()));
        assert!(
            name("alpha") < name("Beta"),
            "text folds case like Cell::compare"
        );

        let ages = SortKey::by(SortField::Cell(CellSortKey::new(
            ColumnId::new(ColumnId::AGE),
            provider,
        )));
        let age = |created: &str| {
            ages.value_of(&StoreObject::Resource(
                pod().name("p").created(created).build(),
            ))
        };
        assert!(
            age("2026-01-02T00:00:00Z") < age("2025-01-01T00:00:00Z"),
            "the younger object has the smaller age"
        );
    }

    #[test]
    fn cell_keys_compare_by_column_and_provider_instance() {
        let a: Arc<dyn ColumnProvider> = Arc::new(CoreColumns::new());
        let b: Arc<dyn ColumnProvider> = Arc::new(CoreColumns::new());
        let key =
            |p: &Arc<dyn ColumnProvider>, c: &str| CellSortKey::new(ColumnId::new(c), p.clone());
        assert_eq!(key(&a, "ready"), key(&a, "ready"));
        assert_ne!(key(&a, "ready"), key(&a, "status"));
        assert_ne!(key(&a, "ready"), key(&b, "ready"));
        assert_eq!(
            format!("{:?}", key(&a, "ready")),
            "CellSortKey(ColumnId(\"ready\"))"
        );
    }
}
