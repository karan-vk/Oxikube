//! [`TableColumns`]: the [`ColumnProvider`] for a server-side Table feed (ADR 0006), the path for
//! CRDs and every kind the core catalogue does not cover.
//!
//! It maps one feed's `columnDefinitions` (name, type, format, description, priority; for a CRD,
//! its `additionalPrinterColumns`) to [`Column`]s and each row's `cells` to [`Cell`]s. A column
//! with `priority > 0` is flagged `wide`, matching `kubectl get -o wide`. A provider is built per
//! feed, from the columns the feed delivers (they arrive on the first batch and change only with
//! a restart), so it is cheap and holds nothing but the mapping.
//!
//! When the feed fell back to plain objects ([`TableSource::Objects`]) the server's columns are
//! the adapter's stand-ins, so the provider substitutes the generic Name / Namespace / Age
//! columns, which every object can answer from its metadata.

mod sniff;

use std::sync::Arc;

use jiff::Timestamp;
use oxikube_domain::ids::{Gvk, Scope};
use oxikube_domain::{Age, Capabilities};
use oxikube_ports::{TableColumn, TableSource};
use serde_json::Value;

use super::builtin::{meta_cell, scalar, status_tone};
use super::{Align, Cell, Column, ColumnId, ColumnProvider, SortKind};
use crate::store::StoreObject;

/// How a column's cells are read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Read {
    /// From the row's cell at this index, typed by the server column type.
    Cell { index: usize, ty: CellType },
    /// From the object's metadata (`name`, `namespace`, `age`): the generic and synthetic columns.
    Meta,
}

/// The server's OpenAPI type of a column, reduced to what changes how a cell reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CellType {
    Integer,
    Number,
    Date,
    Text,
}

#[derive(Debug, Clone)]
struct Slot {
    read: Read,
    /// Shows the time since creation: recomputed from `metadata.creationTimestamp` so it ticks
    /// between server refreshes, instead of showing the text the server rendered once.
    age: bool,
    /// A status-like column (`Status`, `Phase`, `State`): its text is toned.
    status: bool,
}

/// The [`ColumnProvider`] of one Table feed. See the [module docs](self).
#[derive(Debug, Clone)]
pub struct TableColumns {
    columns: Arc<[Column]>,
    slots: Vec<Slot>,
}

impl TableColumns {
    /// A provider for a feed that delivered `definitions` from `source`.
    ///
    /// `scope` says whether the kind is namespaced. The server's table has no namespace column,
    /// so a namespaced kind gets a synthetic `namespace` column, hidden by default, placed after
    /// the name; the table shows it when several namespaces are selected.
    pub fn new(definitions: &[TableColumn], source: TableSource, scope: Scope) -> Self {
        let (columns, slots) = match source {
            TableSource::Objects => generic(scope),
            TableSource::Server => server(definitions, scope),
        };
        Self {
            columns: columns.into(),
            slots,
        }
    }

    fn column_index(&self, id: &ColumnId) -> Option<usize> {
        self.columns.iter().position(|c| c.id == *id)
    }
}

impl ColumnProvider for TableColumns {
    /// The feed's columns. A Table provider serves one feed, so `kind` and `caps` do not change
    /// the answer.
    fn columns(&self, _kind: &Gvk, _caps: Capabilities) -> Arc<[Column]> {
        self.columns.clone()
    }

    fn cell<'a>(&self, object: &'a StoreObject, column: &ColumnId, now: Timestamp) -> Cell<'a> {
        let Some(i) = self.column_index(column) else {
            return Cell::empty();
        };
        let slot = &self.slots[i];
        let (Read::Cell { index, ty }, StoreObject::Row(row)) = (slot.read, object) else {
            return meta_cell(object.meta(), column, now);
        };
        if slot.age
            && let Some(created) = row.meta.creation
        {
            return Cell::age(Age::between(created, now));
        }
        let Some(value) = row.cells.get(index) else {
            return Cell::empty();
        };
        let cell = typed(value, ty, slot.age);
        if slot.status {
            let tone = status_tone(cell.display());
            return cell.with_tone(tone);
        }
        cell
    }
}

/// A row cell value as a cell, typed by its column. Only strings need the column's type;
/// numbers, booleans and blanks read the same as in the core catalogue.
fn typed(value: &Value, ty: CellType, age_like: bool) -> Cell<'_> {
    match value {
        Value::String(s) if age_like || ty == CellType::Date => {
            Cell::shown(s.as_str(), sniff::age_sort(s))
        }
        Value::String(s) => Cell::shown(s.as_str(), sniff::text_sort(s)),
        other => scalar(other),
    }
}

fn cell_type(def: &TableColumn) -> CellType {
    match def.column_type.as_str() {
        "integer" => CellType::Integer,
        "number" => CellType::Number,
        "date" => CellType::Date,
        _ => CellType::Text,
    }
}

fn server(definitions: &[TableColumn], scope: Scope) -> (Vec<Column>, Vec<Slot>) {
    let mut columns = Vec::with_capacity(definitions.len() + 1);
    let mut slots = Vec::with_capacity(definitions.len() + 1);
    let mut used: Vec<String> = Vec::new();
    let synthesise_namespace = scope == Scope::Namespaced && !has_namespace(definitions);
    for (index, def) in definitions.iter().enumerate() {
        let ty = cell_type(def);
        let age = def.name.eq_ignore_ascii_case("age");
        let status = ["status", "phase", "state"]
            .iter()
            .any(|n| def.name.eq_ignore_ascii_case(n));
        let (align, sort) = match ty {
            CellType::Integer | CellType::Number => (Align::Right, SortKind::Number),
            CellType::Date if age => (Align::Right, SortKind::Age),
            CellType::Date => (Align::Left, SortKind::Time),
            CellType::Text if age => (Align::Right, SortKind::Age),
            CellType::Text => (Align::Left, SortKind::Text),
        };
        columns.push(Column {
            id: ColumnId::new(unique(&mut used, sniff::slug(&def.name))),
            title: Arc::from(def.name.as_str()),
            description: (!def.description.is_empty()).then(|| Arc::from(def.description.as_str())),
            wide: def.priority > 0,
            align,
            sort,
            table_index: Some(index),
        });
        slots.push(Slot {
            read: Read::Cell { index, ty },
            age,
            status,
        });
        // The synthetic namespace column goes right after the name.
        if synthesise_namespace && def.format == "name" {
            columns.push(namespace_column(true));
            slots.push(meta_slot());
            used.push(ColumnId::NAMESPACE.to_owned());
        }
    }
    (columns, slots)
}

fn has_namespace(definitions: &[TableColumn]) -> bool {
    definitions
        .iter()
        .any(|d| d.name.eq_ignore_ascii_case("namespace"))
}

/// Makes `id` unique among `used` by appending `-2`, `-3`, ... (two server columns can share a
/// name, such as two `Ready` columns).
fn unique(used: &mut Vec<String>, id: String) -> String {
    let mut candidate = id.clone();
    let mut n = 2;
    while used.contains(&candidate) {
        candidate = format!("{id}-{n}");
        n += 1;
    }
    used.push(candidate.clone());
    candidate
}

fn meta_slot() -> Slot {
    Slot {
        read: Read::Meta,
        age: false,
        status: false,
    }
}

/// A column read from metadata, with no server definition behind it.
fn meta_column(id: &str, title: &str, wide: bool, align: Align, sort: SortKind) -> Column {
    Column {
        id: ColumnId::new(id),
        title: Arc::from(title),
        description: None,
        wide,
        align,
        sort,
        table_index: None,
    }
}

fn namespace_column(wide: bool) -> Column {
    meta_column(
        ColumnId::NAMESPACE,
        "Namespace",
        wide,
        Align::Left,
        SortKind::Text,
    )
}

/// Name, Namespace (namespaced kinds), Age: what any object answers from its metadata.
fn generic(scope: Scope) -> (Vec<Column>, Vec<Slot>) {
    let mut columns = vec![meta_column(
        ColumnId::NAME,
        "Name",
        false,
        Align::Left,
        SortKind::Text,
    )];
    if scope == Scope::Namespaced {
        columns.push(namespace_column(false));
    }
    columns.push(meta_column(
        ColumnId::AGE,
        "Age",
        false,
        Align::Right,
        SortKind::Age,
    ));
    let slots = columns.iter().map(|_| meta_slot()).collect();
    (columns, slots)
}
