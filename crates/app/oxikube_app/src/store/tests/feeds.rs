//! Feed scripts end to end: initial list, add / modify / delete, coalescing, resync after a
//! disconnect, errors, and the policy's choice of feed.

use std::sync::Arc;
use std::time::Duration;

use oxikube_domain::{ErrorKind, ObjectMeta, OxiError};
use oxikube_ports::{Delta, DeltaBatch, TableBatch, TableColumn, TableRow, TableSource};
use oxikube_testkit::{ResourceCall, TableCall, Timeline, resource};
use serde_json::json;

use super::*;
use crate::store::{FeedKind, FeedState, RowOp, StoreObject};

#[test]
fn first_item_is_a_snapshot_of_the_initial_list_then_ready() {
    let mut h = Harness::with_objects([
        p("b", "web-1", "1"),
        p("a", "web-2", "1"),
        p("a", "db", "1"),
    ]);
    let mut sub = h.store.subscribe(all(pods()));
    let first = next(&mut sub).expect("a first item before the feed lists");
    assert_eq!(first.rows, RowChange::Snapshot(vec![]));
    assert_eq!(first.state, FeedState::Warming);
    assert!(
        next(&mut sub).is_none(),
        "nothing more until the feed answers"
    );

    h.settle();
    let mut m = Mirror::default();
    assert_eq!(m.drain(&mut sub), 1, "the whole list arrives as one item");
    assert!(matches!(m.last_rows(), RowChange::Snapshot(_)));
    assert_eq!(m.names(), ["a/db", "a/web-2", "b/web-1"], "kubectl order");
    assert_eq!(m.last.as_ref().unwrap().state, FeedState::Ready);
    assert_eq!(sub.state(), FeedState::Ready);
    assert_eq!(sub.feed_kind(), FeedKind::Full);
}

#[test]
fn add_modify_delete_arrive_as_one_batch_of_positioned_ops() {
    let mut h = Harness::new();
    h.resources.script().watch.push_ok(timeline(vec![
        batch(vec![Delta::Restarted(vec![
            p("x", "a", "1"),
            p("x", "c", "1"),
        ])]),
        batch(vec![
            Delta::Applied(p("x", "b", "1")),
            Delta::Applied(p("x", "c", "2")),
            Delta::Deleted(p("x", "a", "1")),
        ]),
    ]));
    let mut sub = h.subscribe(all(pods()));
    let mut m = Mirror::default();
    m.drain(&mut sub);
    assert_eq!(m.names(), ["x/a", "x/c"]);

    h.advance(1);
    assert_eq!(m.drain(&mut sub), 1);
    let RowChange::Ops(ops) = m.last_rows().clone() else {
        panic!("expected ops, got {:?}", m.last_rows());
    };
    assert_eq!(ops.len(), 3, "{ops:?}");
    assert!(ops.contains(&RowOp::Remove { index: 0 }), "a was first");
    assert!(ops.iter().any(|o| matches!(o, RowOp::Update { object, .. } if object.meta().resource_version.as_deref() == Some("2"))));
    assert!(
        ops.iter()
            .any(|o| matches!(o, RowOp::Insert { object, .. } if object.name() == "b"))
    );
    assert_eq!(m.names(), ["x/b", "x/c"]);
}

#[test]
fn bursts_are_coalesced_until_the_consumer_polls() {
    let mut h = Harness::new();
    h.resources.script().watch.push_ok(timeline(vec![
        batch(vec![Delta::Restarted(vec![])]),
        batch(vec![Delta::Applied(p("x", "a", "1"))]),
        batch(vec![Delta::Applied(p("x", "b", "1"))]),
        batch(vec![Delta::Applied(p("x", "a", "2"))]),
    ]));
    let mut sub = h.subscribe(all(pods()));
    let mut m = Mirror::default();
    m.drain(&mut sub);
    h.advance(3);
    assert_eq!(m.drain(&mut sub), 1, "three feed batches, one item");
    assert_eq!(m.names(), ["x/a", "x/b"]);
    assert!(next(&mut sub).is_none());
}

#[test]
fn a_relist_after_a_disconnect_resyncs_the_rows() {
    let mut h = Harness::new();
    // The first feed lists a and b and then ends (the connection dropped); the store reopens
    // it after the backoff and the relist replaces the rows.
    h.resources
        .script()
        .watch
        .push_ok(Timeline::immediate([batch(vec![Delta::Restarted(vec![
            p("x", "a", "1"),
            p("x", "b", "1"),
        ])])]));
    h.resources.script().watch.push_ok(
        Timeline::immediate([batch(vec![Delta::Restarted(vec![
            p("x", "b", "1"),
            p("x", "c", "1"),
        ])])])
        .keep_open(),
    );
    let mut sub = h.subscribe(all(pods()));
    let mut m = Mirror::default();
    m.drain(&mut sub);
    assert_eq!(m.names(), ["x/a", "x/b"]);
    assert!(matches!(sub.state(), FeedState::Retrying { .. }));

    h.advance(1);
    m.drain(&mut sub);
    assert_eq!(m.names(), ["x/b", "x/c"]);
    assert_eq!(sub.state(), FeedState::Ready);
    let watches = h
        .resources
        .recorded_calls()
        .into_iter()
        .filter(|c| matches!(c, ResourceCall::Watch { .. }))
        .count();
    assert_eq!(watches, 2, "reopened once");
}

#[test]
fn a_retryable_error_keeps_the_rows_until_the_relist() {
    let mut h = Harness::new();
    h.resources.script().watch.push_ok(
        Timeline::new()
            .ok_at(
                Duration::ZERO,
                batch(vec![Delta::Restarted(vec![p("x", "a", "1")])]),
            )
            .err_at(
                Duration::from_secs(1),
                OxiError::network("connection reset"),
            )
            .ok_at(
                Duration::from_secs(2),
                batch(vec![Delta::Restarted(vec![p("x", "a", "2")])]),
            )
            .keep_open(),
    );
    let mut sub = h.subscribe(all(pods()));
    let mut m = Mirror::default();
    m.drain(&mut sub);
    h.advance(1);
    m.drain(&mut sub);
    assert_eq!(m.names(), ["x/a"], "rows kept while retrying");
    assert!(matches!(
        &m.last.as_ref().unwrap().state,
        FeedState::Retrying { message } if message.contains("reset")
    ));
    h.advance(1);
    m.drain(&mut sub);
    assert_eq!(m.last.as_ref().unwrap().state, FeedState::Ready);
}

#[test]
fn a_forbidden_feed_is_reported_and_not_retried_until_a_new_subscriber() {
    let mut h = Harness::new();
    h.resources
        .script()
        .watch
        .push_err(OxiError::forbidden("pods is forbidden"));
    let mut sub = h.subscribe(all(pods()));
    assert!(
        matches!(sub.state(), FeedState::Forbidden { message } if message.contains("forbidden"))
    );
    h.advance(120);
    let resources = h.resources.clone();
    let watches = move || {
        resources
            .recorded_calls()
            .iter()
            .filter(|c| matches!(c, ResourceCall::Watch { .. }))
            .count()
    };
    assert_eq!(watches(), 1, "terminal errors are not retried on a timer");
    let mut m = Mirror::default();
    m.drain(&mut sub);
    assert!(matches!(
        m.last.as_ref().unwrap().state,
        FeedState::Forbidden { .. }
    ));

    // A second view subscribing retries (the default fallback lists the empty store).
    let other = h.subscribe(all(pods()));
    assert_eq!(watches(), 2);
    assert_eq!(other.state(), FeedState::Ready);
    assert_eq!(sub.state(), FeedState::Ready, "the first view recovers too");
}

#[test]
fn an_unknown_kind_fails_with_its_error_kind() {
    let mut h = Harness::new();
    h.tables.script().table_feed.push_err(OxiError::not_found(
        "the server could not find the requested resource",
    ));
    let sub = h.subscribe(all(widgets()));
    assert!(matches!(
        sub.state(),
        FeedState::Failed {
            kind: ErrorKind::NotFound,
            ..
        }
    ));
}

fn row(ns: &str, name: &str, cells: Vec<serde_json::Value>) -> TableRow {
    let mut meta = ObjectMeta::named(name);
    meta.namespace = Some(ns.into());
    meta.resource_version = Some("1".into());
    TableRow {
        cells,
        meta: Some(meta),
        object: None,
    }
}

#[test]
fn core_kinds_use_reflectors_crds_the_table_api_and_both_yield_the_same_shape() {
    let mut h = Harness::with_objects([p("x", "a", "1"), p("x", "b", "1")]);
    let columns: Arc<[TableColumn]> = Arc::from(vec![TableColumn {
        name: "Name".into(),
        column_type: "string".into(),
        ..TableColumn::default()
    }]);
    h.tables.script().table_feed.push_ok(
        Timeline::immediate([TableBatch {
            columns: Some(columns.clone()),
            rows: DeltaBatch::from_deltas(vec![Delta::Restarted(vec![
                row("x", "b", vec![json!("b")]),
                row("x", "a", vec![json!("a")]),
                TableRow::default(),
            ])]),
            source: TableSource::Server,
        }])
        .keep_open(),
    );

    let mut pods_sub = h.subscribe(all(pods()));
    let mut widgets_sub = h.subscribe(all(widgets()));
    assert_eq!(pods_sub.feed_kind(), FeedKind::Full);
    assert_eq!(widgets_sub.feed_kind(), FeedKind::Table);
    assert!(
        h.resources
            .recorded_calls()
            .iter()
            .any(|c| matches!(c, ResourceCall::Watch { kind, options, .. } if *kind == pods() && !options.metadata_only))
    );
    assert!(
        h.tables
            .recorded_calls()
            .iter()
            .any(|c| matches!(c, TableCall::TableFeed { kind, .. } if *kind == widgets()))
    );

    let (mut a, mut b) = (Mirror::default(), Mirror::default());
    a.drain(&mut pods_sub);
    b.drain(&mut widgets_sub);
    assert_eq!(
        a.names(),
        b.names(),
        "same rows, same order, same Delta shape"
    );
    assert!(matches!(&*a.rows[0], StoreObject::Resource(_)));
    assert_eq!(
        b.rows[0].cells(),
        Some(&[json!("a")][..]),
        "rows keep their cells"
    );
    let widget_columns = b.last.as_ref().unwrap().columns.clone();
    assert_eq!(
        widget_columns.map(|c| c.columns),
        Some(columns),
        "columns delivered"
    );
    assert!(a.last.as_ref().unwrap().columns.is_none());
}

#[test]
fn secrets_are_watched_metadata_only() {
    let secret = resource("v1", "Secret")
        .namespace("x")
        .name("token")
        .field("data", json!({"password": "c2VjcmV0"}))
        .build();
    let mut h = Harness::with_objects([secret]);
    let mut sub = h.subscribe(all(oxikube_domain::ids::Gvk::new("", "v1", "Secret")));
    assert_eq!(sub.feed_kind(), FeedKind::Metadata);
    let mut m = Mirror::default();
    m.drain(&mut sub);
    let cached = m.rows[0].resource().expect("a resource");
    assert!(cached.is_partial());
    assert!(
        cached.json.get("data").is_none(),
        "no Secret data in the cache"
    );
    assert!(!format!("{:?}", m.rows[0]).contains("c2VjcmV0"));
}
