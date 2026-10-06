//! [`TableColumns`] on recorded server Table responses (`tests/fixtures`): a CRD with
//! `additionalPrinterColumns` and the pod table.

use std::sync::Arc;

use jiff::Timestamp;
use oxikube_domain::ids::{Gvk, Scope};
use oxikube_domain::{Capabilities, ObjectMeta, Resource};
use oxikube_ports::{TableColumn, TableSource};
use serde_json::Value;

use super::now;
use crate::columns::{
    Align, Cell, CellSort, Column, ColumnId, ColumnProvider, SortKind, TableColumns, Tone,
};
use crate::store::{StoreObject, TableObject};

const WIDGETS: &str = include_str!("fixtures/widgets.json");
const PODS: &str = include_str!("fixtures/pods.json");

/// Parses a recorded `meta.k8s.io/v1` Table the way the adapter does: its column definitions
/// and, per row, the cells and the embedded object's metadata.
fn recorded(text: &str) -> (Vec<TableColumn>, Vec<StoreObject>) {
    let table: Value = serde_json::from_str(text).unwrap();
    let columns = table["columnDefinitions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| TableColumn {
            name: c["name"].as_str().unwrap().to_owned(),
            column_type: c["type"].as_str().unwrap().to_owned(),
            format: c["format"].as_str().unwrap().to_owned(),
            description: c["description"].as_str().unwrap().to_owned(),
            priority: i32::try_from(c["priority"].as_i64().unwrap()).unwrap(),
        })
        .collect();
    let rows = table["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            let object = row["object"].clone();
            let meta: ObjectMeta = Resource::from_json(object).unwrap().meta;
            StoreObject::Row(TableObject {
                meta,
                cells: row["cells"].as_array().unwrap().clone(),
                object: None,
            })
        })
        .collect();
    (columns, rows)
}

fn gvk() -> Gvk {
    Gvk::new("test.oxikube.dev", "v1", "Widget")
}

fn widgets() -> (TableColumns, Arc<[Column]>, Vec<StoreObject>) {
    let (defs, rows) = recorded(WIDGETS);
    let provider = TableColumns::new(&defs, TableSource::Server, Scope::Namespaced);
    let columns = provider.columns(&gvk(), Capabilities::empty());
    (provider, columns, rows)
}

fn at(provider: &TableColumns, row: &StoreObject, id: &str) -> String {
    provider
        .cell(row, &ColumnId::new(id), now())
        .display()
        .to_owned()
}

#[test]
fn a_crd_shows_its_additional_printer_columns() {
    let (_, columns, _) = widgets();
    let titles: Vec<&str> = columns.iter().map(|c| &*c.title).collect();
    // The server's six columns, plus the synthetic namespace column after Name.
    assert_eq!(
        titles,
        [
            "Name",
            "Namespace",
            "Size",
            "Replicas",
            "Phase",
            "Age",
            "Owner"
        ]
    );
    let ids: Vec<&str> = columns.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "name",
            "namespace",
            "size",
            "replicas",
            "phase",
            "age",
            "owner"
        ]
    );
}

#[test]
fn priority_above_zero_is_the_wide_flag() {
    let (_, columns, _) = widgets();
    let wide: Vec<&str> = columns
        .iter()
        .filter(|c| c.wide)
        .map(|c| c.id.as_str())
        .collect();
    // `Owner` is `priority: 1`; the synthetic namespace column is hidden by default as well.
    assert_eq!(wide, ["namespace", "owner"]);
    assert!(
        columns
            .iter()
            .find(|c| c.id == *"size")
            .unwrap()
            .is_default()
    );
}

#[test]
fn cells_map_from_the_rows() {
    let (provider, _, rows) = widgets();
    let large = &rows[0];
    assert_eq!(at(&provider, large, "name"), "large");
    assert_eq!(at(&provider, large, "namespace"), "oxikube-fixtures");
    assert_eq!(at(&provider, large, "size"), "large");
    assert_eq!(at(&provider, large, "replicas"), "10");
    assert_eq!(at(&provider, large, "phase"), "", "a null cell is blank");
    assert_eq!(at(&provider, large, "owner"), "team-a");
    assert_eq!(at(&provider, &rows[2], "replicas"), "1");
}

#[test]
fn types_decide_alignment_and_sort_kind() {
    let (_, columns, _) = widgets();
    let col = |id: &str| columns.iter().find(|c| c.id == *id).unwrap();
    assert_eq!(col("replicas").align, Align::Right);
    assert_eq!(col("replicas").sort, SortKind::Number);
    assert_eq!(col("age").sort, SortKind::Age);
    assert_eq!(col("size").sort, SortKind::Text);
    assert_eq!(col("owner").description.as_deref().is_some(), true);
}

#[test]
fn table_columns_carry_the_cell_index_the_store_sorts_by() {
    let (_, columns, _) = widgets();
    let index = |id: &str| columns.iter().find(|c| c.id == *id).unwrap().table_index;
    assert_eq!(index("name"), Some(0));
    assert_eq!(index("replicas"), Some(2));
    assert_eq!(index("owner"), Some(5));
    assert_eq!(
        index("namespace"),
        None,
        "synthetic: read from the row's metadata"
    );
}

#[test]
fn integer_cells_sort_by_value() {
    let (provider, _, rows) = widgets();
    let mut by_replicas: Vec<(&str, Cell)> = rows
        .iter()
        .map(|r| {
            (
                r.name(),
                provider.cell(r, &ColumnId::new("replicas"), now()),
            )
        })
        .collect();
    by_replicas.sort_by(|a, b| a.1.compare(&b.1));
    // 1, 3, 10: as text "1" < "10" < "3".
    let names: Vec<&str> = by_replicas.iter().map(|(n, _)| *n).collect();
    assert_eq!(names, ["small", "medium", "large"]);
    assert_eq!(
        provider
            .cell(&rows[0], &ColumnId::new("replicas"), now())
            .sort(),
        CellSort::Int(10)
    );
}

#[test]
fn the_age_column_ticks_from_the_creation_time() {
    let (provider, _, rows) = widgets();
    // The server printed `15h` once; creation was 2026-10-03T23:25:40Z.
    let later: Timestamp = "2026-10-05T23:25:40Z".parse().unwrap();
    let age = |when| provider.cell(&rows[0], &ColumnId::new("age"), when);
    assert_eq!(age(later).display(), "2d");
    assert_eq!(age("2026-10-04T03:25:40Z".parse().unwrap()).display(), "4h");
    assert!(matches!(age(later).sort(), CellSort::Age(_)));
}

#[test]
fn the_pod_table_recovers_numbers_from_string_cells() {
    let (defs, rows) = recorded(PODS);
    let provider = TableColumns::new(&defs, TableSource::Server, Scope::Namespaced);
    let columns = provider.columns(&Gvk::new("", "v1", "Pod"), Capabilities::empty());
    let wide: Vec<&str> = columns
        .iter()
        .filter(|c| c.wide)
        .map(|c| &*c.title)
        .collect();
    assert_eq!(
        wide,
        [
            "Namespace",
            "IP",
            "Node",
            "Nominated Node",
            "Readiness Gates"
        ]
    );
    let first = &rows[0];
    let cell = |id: &str| provider.cell(first, &ColumnId::new(id), now());
    assert_eq!(cell("ready").display(), "1/1");
    assert_eq!(cell("ready").sort(), CellSort::Float(1.0));
    assert_eq!(cell("restarts").sort(), CellSort::Int(0));
    assert_eq!(cell("status").display(), "Running");
    assert_eq!(cell("status").tone(), Tone::Ok);
    assert_eq!(cell("ip").display(), "10.244.0.6");
    assert_eq!(
        cell("ip").sort(),
        CellSort::Text,
        "an address is text, not a number"
    );
    assert_eq!(cell("node").display(), "oxikube-control-plane");
}

#[test]
fn duplicate_server_names_get_distinct_ids() {
    let def = |name: &str| TableColumn {
        name: name.to_owned(),
        column_type: "string".to_owned(),
        ..TableColumn::default()
    };
    let provider = TableColumns::new(
        &[def("Ready"), def("Ready"), def("Created At")],
        TableSource::Server,
        Scope::Cluster,
    );
    let ids: Vec<String> = provider
        .columns(&gvk(), Capabilities::empty())
        .iter()
        .map(|c| c.id.to_string())
        .collect();
    assert_eq!(ids, ["ready", "ready-2", "created-at"]);
}

#[test]
fn a_cluster_scoped_kind_gets_no_namespace_column() {
    let (defs, _) = recorded(WIDGETS);
    let provider = TableColumns::new(&defs, TableSource::Server, Scope::Cluster);
    let columns = provider.columns(&gvk(), Capabilities::empty());
    assert!(columns.iter().all(|c| c.id != *"namespace"));
}

#[test]
fn a_feed_that_fell_back_to_objects_gets_the_generic_columns() {
    // What the adapter synthesises when the server ignores the Table Accept header.
    let defs = vec![
        TableColumn {
            name: "Name".into(),
            column_type: "string".into(),
            format: "name".into(),
            ..TableColumn::default()
        },
        TableColumn {
            name: "Created At".into(),
            column_type: "date".into(),
            ..TableColumn::default()
        },
    ];
    let provider = TableColumns::new(&defs, TableSource::Objects, Scope::Namespaced);
    let columns = provider.columns(&gvk(), Capabilities::empty());
    let ids: Vec<&str> = columns.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, ["name", "namespace", "age"]);
    assert!(columns.iter().all(|c| !c.wide));

    let mut meta = ObjectMeta::named("large");
    meta.namespace = Some("oxikube-fixtures".into());
    meta.creation = Some("2026-01-01T00:00:00Z".parse().unwrap());
    let row = StoreObject::Row(TableObject {
        meta,
        cells: vec![Value::from("large"), Value::from("2026-01-01T00:00:00Z")],
        object: None,
    });
    assert_eq!(at(&provider, &row, "name"), "large");
    assert_eq!(at(&provider, &row, "namespace"), "oxikube-fixtures");
    assert_eq!(at(&provider, &row, "age"), "27h");
}

#[test]
fn an_unknown_column_or_a_short_row_is_blank() {
    let (provider, _, rows) = widgets();
    assert_eq!(at(&provider, &rows[0], "nope"), "");
    let short = StoreObject::Row(TableObject {
        meta: ObjectMeta::named("short"),
        cells: vec![Value::from("short")],
        object: None,
    });
    assert_eq!(at(&provider, &short, "replicas"), "");
    assert_eq!(at(&provider, &short, "name"), "short");
}
