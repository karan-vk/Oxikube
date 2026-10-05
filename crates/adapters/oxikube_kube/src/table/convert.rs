//! Wire Table (or plain list) to the port's [`Table`](oxikube_ports::Table) pieces.
//!
//! A Table converts column for column and row for row. A plain list (the server ignored the
//! Table `Accept` header) becomes the columns the apiserver's own default table convertor
//! produces for kinds without printer columns, `Name` and `Created At`, with
//! [`TableSource::Objects`] so a `ColumnProvider` knows to substitute its own (ADR 0006).

use std::sync::Arc;

use oxikube_domain::{ObjectMeta, OxiResult, Resource};
use oxikube_ports::{IncludeObject, TableColumn, TableRow, TableSource};
use serde_json::{Value, json};

use super::wire::{self, ColumnDefinition};
use crate::resources::bad_object;

/// One decoded page, whichever way the server answered.
pub(crate) struct Page {
    /// Columns: the server's, or the synthesised fallback pair.
    pub(crate) columns: Arc<[TableColumn]>,
    /// Which of the two.
    pub(crate) source: TableSource,
    /// The rows of this page.
    pub(crate) rows: Vec<TableRow>,
    /// Token for the next page; `None` on the last.
    pub(crate) continue_token: Option<String>,
    /// The collection's resource version.
    pub(crate) resource_version: Option<String>,
}

/// Converts one list response.
pub(crate) fn page(table: wire::Table, include: IncludeObject) -> OxiResult<Page> {
    let source = if table.is_table() {
        TableSource::Server
    } else {
        TableSource::Objects
    };
    let wire::Table {
        metadata,
        column_definitions,
        rows,
        items,
        ..
    } = table;
    let (columns, rows) = match source {
        TableSource::Server => (columns(column_definitions), table_rows(rows, include)?),
        TableSource::Objects => (
            object_columns(),
            items
                .into_iter()
                .map(|item| object_row(item, include))
                .collect::<OxiResult<_>>()?,
        ),
    };
    Ok(Page {
        columns,
        source,
        rows,
        continue_token: metadata.continue_token.filter(|t| !t.is_empty()),
        resource_version: metadata.resource_version.filter(|v| !v.is_empty()),
    })
}

/// Server column definitions to port columns.
pub(crate) fn columns(definitions: Vec<ColumnDefinition>) -> Arc<[TableColumn]> {
    definitions
        .into_iter()
        .map(|d| TableColumn {
            name: d.name,
            column_type: d.column_type,
            format: d.format,
            description: d.description,
            priority: d.priority,
        })
        .collect()
}

/// Server rows to port rows.
pub(crate) fn table_rows(
    rows: Vec<wire::TableRow>,
    include: IncludeObject,
) -> OxiResult<Vec<TableRow>> {
    rows.into_iter()
        .map(|row| table_row(row, include))
        .collect()
}

fn table_row(row: wire::TableRow, include: IncludeObject) -> OxiResult<TableRow> {
    let wire::TableRow { cells, object } = row;
    let (meta, object) = match object {
        None => (None, None),
        Some(object) => split_object(object, include)?,
    };
    Ok(TableRow {
        cells,
        meta,
        object,
    })
}

/// The fallback columns: what the apiserver's default table convertor sends for a kind
/// without printer columns.
pub(crate) fn object_columns() -> Arc<[TableColumn]> {
    Arc::from(vec![
        TableColumn {
            name: "Name".into(),
            column_type: "string".into(),
            format: "name".into(),
            description: "Name must be unique within a namespace.".into(),
            priority: 0,
        },
        TableColumn {
            name: "Created At".into(),
            column_type: "date".into(),
            format: String::new(),
            description: "CreationTimestamp is the server time when this object was created."
                .into(),
            priority: 0,
        },
    ])
}

/// One plain object (fallback) as a row of [`object_columns`].
pub(crate) fn object_row(object: Value, include: IncludeObject) -> OxiResult<TableRow> {
    let metadata = object.get("metadata");
    let field = |name: &str| {
        metadata
            .and_then(|m| m.get(name))
            .cloned()
            .unwrap_or(Value::Null)
    };
    let cells = vec![field("name"), field("creationTimestamp")];
    let (meta, object) = split_object(object, include)?;
    Ok(TableRow {
        cells,
        meta,
        object,
    })
}

/// Row identity and embedded object from a row's object, per `include`: the whole object is
/// kept only for [`IncludeObject::Object`], so `Metadata` rows hold no copy of it.
fn split_object(
    mut object: Value,
    include: IncludeObject,
) -> OxiResult<(Option<ObjectMeta>, Option<Value>)> {
    match include {
        IncludeObject::None => Ok((None, None)),
        IncludeObject::Metadata => {
            let metadata = object.get_mut("metadata").map(Value::take);
            Ok((Some(meta_of(metadata)?), None))
        }
        IncludeObject::Object => {
            let metadata = object.get("metadata").cloned();
            Ok((Some(meta_of(metadata)?), Some(object)))
        }
    }
}

/// Server `metadata` JSON to the domain's typed [`ObjectMeta`], through the domain's own
/// parser (it only parses `metadata` as part of an object; the type fields are placeholders).
fn meta_of(metadata: Option<Value>) -> OxiResult<ObjectMeta> {
    let object = json!({
        "apiVersion": "v1",
        "kind": "Row",
        "metadata": metadata.unwrap_or(Value::Null),
    });
    Resource::from_json(object)
        .map(|resource| resource.meta)
        .map_err(|e| bad_object("table row", e))
}
