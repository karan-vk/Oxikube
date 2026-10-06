//! A custom resource table from the server's Table response: printer columns and values, wide
//! columns hidden, and rows that follow the Table feed live, filtered and sorted like any kind.

use std::sync::Arc;

use futures::executor::block_on;
use gpui::TestAppContext;
use oxikube_app::ColumnId;
use oxikube_domain::ObjectMeta;
use oxikube_ports::{Delta, DeltaBatch, TableBatch, TableColumn, TableRow, TableSource};
use oxikube_testkit::{TICK, TableCall, Timeline};
use serde_json::json;

use super::cell;
use crate::crds::tests::fixture::{batch, columns, widget_kind};
use crate::table::tests::fixture::{Fixture, cluster};

fn printer_columns() -> Arc<[TableColumn]> {
    columns(&[
        ("Name", "string", 0),
        ("Size", "string", 0),
        ("Replicas", "integer", 0),
        ("Age", "date", 0),
        ("Owner", "string", 1),
    ])
}

fn row(name: &str, size: &str, replicas: i64) -> TableRow {
    let mut meta = ObjectMeta::named(name);
    meta.namespace = Some("shop".into());
    meta.resource_version = Some("2".into());
    TableRow {
        cells: vec![
            json!(name),
            json!(size),
            json!(replicas),
            json!("1d"),
            json!("team-a"),
        ],
        meta: Some(meta),
        object: None,
    }
}

/// A later batch of the feed: rows only, the columns unchanged.
fn rows_only(deltas: Vec<Delta<TableRow>>) -> TableBatch {
    TableBatch {
        columns: None,
        rows: DeltaBatch::from_deltas(deltas),
        source: TableSource::Server,
    }
}

#[gpui::test]
fn a_custom_resource_table_comes_from_the_table_response_and_follows_its_feed(
    cx: &mut TestAppContext,
) {
    let mut f = Fixture::new(cx);
    let kind = widget_kind("v1", true);
    let ports = f.ports();
    ports.discovery.set_kinds([kind.clone()]);
    let first = batch(
        TableSource::Server,
        printer_columns(),
        &[
            (
                Some("shop"),
                "w-1",
                vec![
                    json!("w-1"),
                    json!("small"),
                    json!(1),
                    json!("2d"),
                    json!("team-a"),
                ],
            ),
            (
                Some("shop"),
                "w-2",
                vec![
                    json!("w-2"),
                    json!("large"),
                    json!(7),
                    json!("1d"),
                    json!("team-b"),
                ],
            ),
        ],
    );
    ports.tables.script().table_feed.push_ok(
        Timeline::new()
            .ok_at(std::time::Duration::ZERO, first)
            // Tick 1: a new widget, and w-1 resized.
            .ok_at(
                TICK,
                rows_only(vec![
                    Delta::Applied(row("w-3", "medium", 3)),
                    Delta::Applied(row("w-1", "huge", 20)),
                ]),
            )
            // Tick 2: w-2 is deleted.
            .ok_at(
                TICK * 2,
                rows_only(vec![Delta::Deleted(row("w-2", "large", 7))]),
            )
            .keep_open(),
    );
    block_on(f.sessions.connect(&cluster())).expect("connect");
    f.vcx.run_until_parked();
    let table = f.open(kind.clone());

    // The server's printer columns are the table's columns; the priority-1 `Owner` is wide.
    assert_eq!(f.names(&table), ["w-1", "w-2"]);
    let mut shown: Vec<String> = f.vcx.update(|_, cx| {
        table.read(cx).read_rows(cx, |d| {
            d.layout()
                .visible_ids()
                .iter()
                .map(ToString::to_string)
                .collect()
        })
    });
    shown.sort();
    assert_eq!(shown, ["age", "name", "replicas", "size"]);
    assert_eq!(cell(&mut f, &table, "w-2", "size"), "large");
    assert_eq!(
        cell(&mut f, &table, "w-2", "owner"),
        "team-b",
        "wide columns still have values"
    );
    assert!(!table.read_with(&f.vcx, |t, _| t.basic_columns()));
    let calls = ports.tables.recorded_calls();
    assert!(
        matches!(&calls[..], [TableCall::TableFeed { kind: k, .. }] if *k == kind.gvk),
        "a custom kind is fed by the Table API, not a reflector: {calls:?}"
    );

    // Sorted by a numeric printer column, then the feed moves rows.
    f.update(&table, |t, cx| {
        t.sort_by(Some((ColumnId::new("replicas"), true)), cx)
    });
    assert_eq!(f.names(&table), ["w-2", "w-1"]);

    ports.tables.clock().advance(TICK);
    f.settle();
    assert_eq!(
        f.names(&table),
        ["w-1", "w-2", "w-3"],
        "w-1 has 20 replicas now"
    );
    assert_eq!(cell(&mut f, &table, "w-1", "size"), "huge");
    assert_eq!(cell(&mut f, &table, "w-3", "replicas"), "3");

    ports.tables.clock().advance(TICK);
    f.settle();
    assert_eq!(f.names(&table), ["w-1", "w-3"]);
}
