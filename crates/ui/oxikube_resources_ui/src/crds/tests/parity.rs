//! "Identical columns" (E12-S17): a custom resource table shows what `kubectl get` shows. The
//! Table response recorded from the kind fixtures (`oxikube_testkit::fixtures::tables::widgets`)
//! goes through the table as the feed delivers it, and the rendered header and cell text are
//! compared with the response's own `columnDefinitions` and `cells`: the server did the
//! formatting, the table must not reformat a value.

use std::sync::Arc;

use futures::executor::block_on;
use gpui::{Entity, TestAppContext};
use jiff::Timestamp;
use oxikube_app::ColumnId;
use oxikube_domain::Resource;
use oxikube_domain::ids::Gvk;
use oxikube_domain::kinds::{ResourceKind, VerbSet};
use oxikube_ports::{Delta, DeltaBatch, TableBatch, TableColumn, TableRow, TableSource};
use oxikube_testkit::Timeline;
use oxikube_testkit::fixtures::tables;
use serde_json::Value;

use crate::table::ResourceTable;
use crate::table::tests::fixture::{Fixture, cluster};

fn widget_kind() -> ResourceKind {
    ResourceKind {
        gvk: Gvk::new("test.oxikube.dev", "v1", "Widget"),
        preferred: true,
        plural: "widgets".into(),
        singular: "widget".into(),
        short_names: vec!["wdg".into()],
        categories: Vec::new(),
        verbs: VerbSet::from_names(["get", "list", "watch"]),
        namespaced: true,
    }
}

/// The recorded Table as the adapter hands it up: the column definitions, and per row the cells
/// and the metadata of the embedded `PartialObjectMetadata`.
fn recorded(table: &Value) -> TableBatch {
    let columns: Arc<[TableColumn]> = table["columnDefinitions"]
        .as_array()
        .expect("columns")
        .iter()
        .map(|c| TableColumn {
            name: c["name"].as_str().unwrap().to_owned(),
            column_type: c["type"].as_str().unwrap().to_owned(),
            format: c["format"].as_str().unwrap_or("").to_owned(),
            description: c["description"].as_str().unwrap_or("").to_owned(),
            priority: i32::try_from(c["priority"].as_i64().unwrap()).unwrap(),
        })
        .collect();
    let rows = table["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .map(|row| TableRow {
            cells: row["cells"].as_array().unwrap().clone(),
            meta: Some(Resource::from_json(row["object"].clone()).unwrap().meta),
            object: None,
        })
        .collect();
    TableBatch {
        columns: Some(columns),
        rows: DeltaBatch::from_deltas(vec![Delta::Restarted(rows)]),
        source: TableSource::Server,
    }
}

/// What `kubectl get` prints for a cell value: strings as they are, numbers and booleans as
/// JSON, `null` blank.
fn printed(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn open(f: &mut Fixture) -> Entity<ResourceTable> {
    let ports = f.ports();
    ports.discovery.set_kinds([widget_kind()]);
    block_on(f.sessions.connect(&cluster())).expect("connect");
    f.vcx.run_until_parked();
    ports
        .tables
        .script()
        .table_feed
        .push_ok(Timeline::immediate([recorded(&tables::widgets())]).keep_open());
    f.open(widget_kind())
}

#[gpui::test]
fn a_custom_resource_table_shows_the_servers_columns_and_cells_verbatim(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    let table = open(&mut f);
    let response = tables::widgets();
    let definitions = response["columnDefinitions"].as_array().unwrap();
    let rows = response["rows"].as_array().unwrap();

    // The header: the server's default columns (`priority` 0), in the server's order and with its
    // titles; the wide one (`Owner`, priority 1) is in the column chooser but hidden.
    let (headers, chooser) = f.vcx.update(|_, cx| {
        table.read(cx).read_rows(cx, |d| {
            let layout = d.layout();
            let headers: Vec<String> = (0..layout.visible_len())
                .filter_map(|ix| layout.visible(ix))
                .filter(|c| c.id != ColumnId::NAMESPACE)
                .map(|c| c.title.to_string())
                .collect();
            let chooser: Vec<(String, bool)> = layout
                .columns()
                .map(|(c, shown)| (c.title.to_string(), shown))
                .collect();
            (headers, chooser)
        })
    });
    let defaults: Vec<&str> = definitions
        .iter()
        .filter(|c| c["priority"] == 0)
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    assert_eq!(headers, defaults);
    for wide in definitions
        .iter()
        .filter(|c| c["priority"].as_i64() > Some(0))
    {
        let title = wide["name"].as_str().unwrap();
        assert!(
            chooser.contains(&(title.to_owned(), false)),
            "{title} is offered, hidden: {chooser:?}"
        );
    }

    // Every cell, default and wide, is the server's text. `Age` is the one the table keeps
    // ticking from `creationTimestamp`; at the moment the response was recorded it reads the same.
    let recorded_at: Timestamp = "2026-10-11T11:40:00Z".parse().unwrap();
    let names: Vec<String> = f.names(&table);
    let expected_names: Vec<String> = rows.iter().map(|r| printed(&r["cells"][0])).collect();
    assert_eq!(names, expected_names);
    let shown = f.vcx.update(|_, cx| {
        table.read(cx).read_rows(cx, |d| {
            let ids: Vec<ColumnId> = d
                .layout()
                .columns()
                .map(|(c, _)| c.id.clone())
                .filter(|id| *id != ColumnId::NAMESPACE)
                .collect();
            (0..rows.len())
                .map(|row| {
                    let object = d.row(row).expect("a row");
                    ids.iter()
                        .map(|id| {
                            d.provider()
                                .cell(object, id, recorded_at)
                                .display()
                                .to_owned()
                        })
                        .collect::<Vec<String>>()
                })
                .collect::<Vec<_>>()
        })
    });
    for (ix, row) in rows.iter().enumerate() {
        let expected: Vec<String> = row["cells"]
            .as_array()
            .unwrap()
            .iter()
            .map(printed)
            .collect();
        // Default columns first, then the wide ones: the server's order within each.
        let mut ordered: Vec<String> = Vec::new();
        for wide in [false, true] {
            for (def, cell) in definitions.iter().zip(&expected) {
                if (def["priority"].as_i64() > Some(0)) == wide {
                    ordered.push(cell.clone());
                }
            }
        }
        assert_eq!(shown[ix], ordered, "row {ix}");
    }
}
