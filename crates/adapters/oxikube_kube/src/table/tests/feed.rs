//! The live feed against a scripted server: first restart, coalesced watch batches, 410
//! and column changes, polling, the plain-JSON fallback, errors and teardown. Paused tokio
//! time: sleeps and the refresh interval elapse instantly once every task is idle.

use std::time::Duration;

use futures::StreamExt;
use oxikube_domain::ErrorKind;
use oxikube_domain::OxiResult;
use oxikube_ports::{
    Delta, IncludeObject, ListOptions, TableBatch, TableFeed, TableFeedPort, TableOptions,
    TableRow, TableSource,
};
use serde_json::json;

use super::harness::*;
use crate::fake_api::{FakeApi, status_body};
use crate::table::{TABLE_ACCEPT, TableConfig};

/// The next feed item; fails (instead of hanging) when none arrives within an hour of
/// paused time.
async fn next(feed: &mut TableFeed) -> Option<OxiResult<TableBatch>> {
    tokio::time::timeout(Duration::from_secs(3600), feed.next())
        .await
        .expect("the feed produced nothing")
}

async fn next_ok(feed: &mut TableFeed) -> TableBatch {
    next(feed).await.expect("feed ended").expect("feed error")
}

/// Lets the feed task run until `done` holds (bounded).
async fn settle(mut done: impl FnMut() -> bool) {
    for _ in 0..200 {
        if done() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("condition not reached");
}

fn summary(batch: &TableBatch) -> Vec<String> {
    batch
        .rows
        .deltas
        .iter()
        .map(|delta| match delta {
            Delta::Applied(row) => format!("applied {}", key(row)),
            Delta::Deleted(row) => format!("deleted {}", key(row)),
            Delta::Restarted(rows) => {
                let names: Vec<_> = rows.iter().map(key).collect();
                format!("restarted [{}]", names.join(" "))
            }
        })
        .collect()
}

fn key(row: &TableRow) -> String {
    let meta = row.meta.as_ref().expect("keyed row");
    format!(
        "{}@{}",
        meta.name,
        meta.resource_version.as_deref().unwrap_or("-")
    )
}

async fn open(api: &FakeApi, gvk: &oxikube_domain::ids::Gvk, options: TableOptions) -> TableFeed {
    adapter(api, TableConfig::default())
        .table_feed(gvk, Some(NS), &options)
        .await
        .expect("open feed")
}

#[tokio::test(start_paused = true)]
async fn the_first_batch_restarts_with_columns_then_watch_events_coalesce() {
    let api = server();
    api.reply(
        WIDGETS,
        200,
        widget_list(&[("a", "1", 1), ("b", "1", 1)], "100", None),
    );
    api.reply_watch(
        WIDGETS,
        200,
        &[
            event("MODIFIED", widget_event_table("a", "101", 2, true)),
            event("ADDED", widget_event_table("c", "102", 1, false)),
            event("DELETED", widget_event_table("b", "103", 1, false)),
            bookmark("104"),
        ],
    );
    let mut feed = open(&api, &widget_gvk(), TableOptions::default()).await;

    let first = next_ok(&mut feed).await;
    assert_eq!(summary(&first), ["restarted [a@1 b@1]"]);
    let columns = first.columns.expect("columns on the first batch");
    assert_eq!(columns.len(), 6);
    assert_eq!(first.source, TableSource::Server);
    assert_eq!(first.rows.resource_version.as_deref(), Some("100"));

    let batch = next_ok(&mut feed).await;
    assert_eq!(
        summary(&batch),
        ["applied a@101", "applied c@102", "deleted b@103"]
    );
    assert_eq!(batch.columns, None);
    assert_eq!(batch.rows.resource_version.as_deref(), Some("104"));
    if let Delta::Applied(row) = &batch.rows.deltas[0] {
        assert_eq!(row.cells[2], json!(2));
    }

    let watch = api.requests().into_iter().find(|r| r.is_watch()).unwrap();
    assert_eq!(watch.accept.as_deref(), Some(TABLE_ACCEPT));
    assert_eq!(
        query_of(&api, WIDGETS, true, 0),
        "watch=true&timeoutSeconds=290&allowWatchBookmarks=true&resourceVersion=100&includeObject=Metadata"
    );
    // The server closed the first connection: the feed watches again from the bookmark.
    settle(|| watches(&api, WIDGETS) == 2).await;
    assert!(query_of(&api, WIDGETS, true, 1).contains("resourceVersion=104"));
    assert_eq!(lists(&api, WIDGETS), 1, "a closed watch does not re-list");
}

#[tokio::test(start_paused = true)]
async fn watch_bursts_are_split_at_max_batch() {
    let api = server();
    api.reply(WIDGETS, 200, widget_list(&[], "1", None));
    api.reply_watch(
        WIDGETS,
        200,
        &[
            event("ADDED", widget_event_table("a", "2", 1, true)),
            event("ADDED", widget_event_table("b", "3", 1, false)),
            event("ADDED", widget_event_table("c", "4", 1, false)),
        ],
    );
    let config = TableConfig {
        max_batch: 2,
        ..TableConfig::default()
    };
    let mut feed = adapter(&api, config)
        .table_feed(&widget_gvk(), Some(NS), &TableOptions::default())
        .await
        .unwrap();
    assert_eq!(summary(&next_ok(&mut feed).await), ["restarted []"]);
    assert_eq!(
        summary(&next_ok(&mut feed).await),
        ["applied a@2", "applied b@3"]
    );
    assert_eq!(summary(&next_ok(&mut feed).await), ["applied c@4"]);
}

#[tokio::test(start_paused = true)]
async fn an_expired_version_relists_and_sends_only_the_diff() {
    let api = server();
    api.reply(
        WIDGETS,
        200,
        widget_list(&[("a", "1", 1), ("b", "1", 1), ("c", "1", 1)], "100", None),
    );
    api.reply(
        WIDGETS,
        200,
        widget_list(&[("a", "1", 1), ("b", "2", 5), ("d", "1", 1)], "200", None),
    );
    api.reply_watch(WIDGETS, 200, &[error_event(410, "Expired")]);
    let mut feed = open(&api, &widget_gvk(), TableOptions::default()).await;
    next_ok(&mut feed).await;

    let batch = next_ok(&mut feed).await;
    assert_eq!(
        summary(&batch),
        ["applied b@2", "applied d@1", "deleted c@1"]
    );
    assert_eq!(batch.columns, None, "same columns: a diff, not a restart");
    settle(|| watches(&api, WIDGETS) == 2).await;
    assert!(query_of(&api, WIDGETS, true, 1).contains("resourceVersion=200"));
}

#[tokio::test(start_paused = true)]
async fn changed_columns_restart_the_feed() {
    let api = server();
    api.reply(WIDGETS, 200, widget_list(&[("a", "1", 1)], "100", None));
    let mut narrower = widget_list(&[("a", "2", 1)], "200", None);
    narrower["columnDefinitions"].as_array_mut().unwrap().pop();
    api.reply(WIDGETS, 200, narrower.clone());
    let mut changed = widget_event_table("a", "2", 1, true);
    changed["columnDefinitions"] = narrower["columnDefinitions"].clone();
    api.reply_watch(WIDGETS, 200, &[event("MODIFIED", changed)]);
    let mut feed = open(&api, &widget_gvk(), TableOptions::default()).await;
    next_ok(&mut feed).await;

    let batch = next_ok(&mut feed).await;
    assert_eq!(summary(&batch), ["restarted [a@2]"]);
    assert_eq!(batch.columns.expect("new columns").len(), 5);
}

#[tokio::test(start_paused = true)]
async fn a_kind_without_watch_is_polled_on_the_refresh_interval() {
    let api = server();
    api.reply(GADGETS, 200, widget_list(&[("a", "1", 1)], "1", None));
    api.reply(GADGETS, 200, widget_list(&[("a", "1", 1)], "2", None));
    api.reply(
        GADGETS,
        200,
        widget_list(&[("a", "1", 1), ("b", "1", 1)], "3", None),
    );
    let mut feed = open(&api, &gadget_gvk(), TableOptions::default()).await;
    let started = tokio::time::Instant::now();
    next_ok(&mut feed).await;

    // The second list is unchanged and sends nothing; the third adds `b`.
    let batch = next_ok(&mut feed).await;
    assert_eq!(summary(&batch), ["applied b@1"]);
    assert!(started.elapsed() >= Duration::from_secs(60));
    assert_eq!(lists(&api, GADGETS), 3);
    assert_eq!(watches(&api, GADGETS), 0);
}

#[tokio::test(start_paused = true)]
async fn a_watch_answered_without_tables_switches_to_polling() {
    let api = server();
    api.reply(WIDGETS, 200, widget_list(&[("a", "1", 1)], "100", None));
    api.reply(WIDGETS, 200, widget_list(&[("a", "2", 3)], "200", None));
    let plain = json!({"apiVersion": "test.oxikube.dev/v1", "kind": "Widget",
        "metadata": {"name": "a", "namespace": NS, "uid": "uid-a", "resourceVersion": "2"}});
    api.reply_watch(WIDGETS, 200, &[event("MODIFIED", plain)]);
    let mut feed = open(&api, &widget_gvk(), TableOptions::default()).await;
    next_ok(&mut feed).await;

    let batch = next_ok(&mut feed).await;
    assert_eq!(summary(&batch), ["applied a@2"]);
    assert_eq!(batch.source, TableSource::Server);
    assert_eq!(watches(&api, WIDGETS), 1, "no second watch once polling");
}

#[tokio::test(start_paused = true)]
async fn a_refused_watch_switches_to_polling() {
    let api = server();
    api.reply(WIDGETS, 200, widget_list(&[("a", "1", 1)], "100", None));
    api.reply(
        WIDGETS,
        200,
        widget_list(&[("a", "1", 1), ("b", "1", 1)], "200", None),
    );
    api.reply_watch(
        WIDGETS,
        403,
        &[status_body(403, "Forbidden", "cannot watch")],
    );
    let mut feed = open(&api, &widget_gvk(), TableOptions::default()).await;
    next_ok(&mut feed).await;
    assert_eq!(summary(&next_ok(&mut feed).await), ["applied b@1"]);
    assert_eq!(watches(&api, WIDGETS), 1);
}

#[tokio::test(start_paused = true)]
async fn the_fallback_feed_watches_plain_objects() {
    let api = server();
    api.reply(WIDGETS, 200, widgets_plain());
    let modified = json!({"apiVersion": "test.oxikube.dev/v1", "kind": "Widget",
        "metadata": {"name": "small", "namespace": NS, "uid": "u-small", "resourceVersion": "999",
                     "creationTimestamp": "2026-10-03T23:25:40Z"},
        "spec": {"size": "small", "replicas": 4}});
    api.reply_watch(WIDGETS, 200, &[event("MODIFIED", modified)]);
    let mut feed = open(&api, &widget_gvk(), TableOptions::default()).await;

    let first = next_ok(&mut feed).await;
    assert_eq!(first.source, TableSource::Objects);
    let columns = first.columns.unwrap();
    assert_eq!(
        columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
        ["Name", "Created At"]
    );

    let batch = next_ok(&mut feed).await;
    assert_eq!(batch.source, TableSource::Objects);
    assert_eq!(summary(&batch), ["applied small@999"]);
    let Delta::Applied(row) = &batch.rows.deltas[0] else {
        unreachable!()
    };
    assert_eq!(row.cells, [json!("small"), json!("2026-10-03T23:25:40Z")]);
    assert_eq!(row.object, None);
}

#[tokio::test(start_paused = true)]
async fn the_first_list_pages_with_the_limit_and_errors_surface_from_table_feed() {
    let api = server();
    api.reply(
        WIDGETS,
        200,
        widget_list(&[("a", "1", 1), ("b", "1", 1)], "100", Some("t1")),
    );
    api.reply(WIDGETS, 200, widget_list(&[("c", "1", 1)], "100", None));
    let options = TableOptions::default().list(ListOptions::default().limit(2).labels("x=y"));
    let mut feed = open(&api, &widget_gvk(), options).await;
    assert_eq!(
        summary(&next_ok(&mut feed).await),
        ["restarted [a@1 b@1 c@1]"]
    );
    assert_eq!(
        query_of(&api, WIDGETS, false, 0),
        "labelSelector=x=y&limit=2&includeObject=Metadata"
    );
    assert_eq!(
        query_of(&api, WIDGETS, false, 1),
        "labelSelector=x=y&limit=2&continue=t1&includeObject=Metadata"
    );
    assert!(query_of(&api, WIDGETS, true, 0).contains("labelSelector=x=y"));

    let denied = server();
    denied.reply(WIDGETS, 403, status_body(403, "Forbidden", "no"));
    let err = adapter(&denied, TableConfig::default())
        .table_feed(&widget_gvk(), Some(NS), &TableOptions::default())
        .await
        .err()
        .expect("forbidden");
    assert_eq!(err.kind(), ErrorKind::Forbidden);
}

#[tokio::test(start_paused = true)]
async fn the_default_page_size_applies_without_a_limit() {
    let api = server();
    api.reply(GADGETS, 200, widget_list(&[], "1", None));
    let _feed = open(&api, &gadget_gvk(), TableOptions::default()).await;
    assert_eq!(
        query_of(&api, GADGETS, false, 0),
        "limit=500&includeObject=Metadata"
    );
}

#[tokio::test(start_paused = true)]
async fn retryable_failures_are_reported_and_the_feed_recovers() {
    let api = server();
    api.reply(WIDGETS, 200, widget_list(&[("a", "1", 1)], "100", None));
    api.reply(WIDGETS, 503, status_body(503, "ServiceUnavailable", "busy"));
    api.reply(WIDGETS, 200, widget_list(&[("a", "2", 1)], "200", None));
    // A connection that closes at once without an event is a failure, not a quiet watch.
    api.reply_watch(WIDGETS, 200, &[]);
    let mut feed = open(&api, &widget_gvk(), TableOptions::default()).await;
    next_ok(&mut feed).await;

    let watch_failed = next(&mut feed).await.unwrap().unwrap_err();
    assert!(watch_failed.is_retryable());
    let list_failed = next(&mut feed).await.unwrap().unwrap_err();
    assert_eq!(list_failed.kind(), ErrorKind::Network);
    assert!(list_failed.is_retryable());
    assert_eq!(summary(&next_ok(&mut feed).await), ["applied a@2"]);
}

#[tokio::test(start_paused = true)]
async fn a_non_retryable_error_is_the_last_item() {
    let api = server();
    api.reply(WIDGETS, 200, widget_list(&[("a", "1", 1)], "100", None));
    api.reply(WIDGETS, 403, status_body(403, "Forbidden", "role revoked"));
    api.reply_watch(WIDGETS, 200, &[error_event(500, "InternalError")]);
    let mut feed = open(&api, &widget_gvk(), TableOptions::default()).await;
    next_ok(&mut feed).await;
    assert!(next(&mut feed).await.unwrap().is_err());
    let last = next(&mut feed).await.unwrap().unwrap_err();
    assert_eq!(last.kind(), ErrorKind::Forbidden);
    assert!(next(&mut feed).await.is_none(), "the feed ends");
}

#[tokio::test(start_paused = true)]
async fn rows_without_identity_are_polled_and_restarted() {
    let api = server();
    let mut bare = widget_list(&[("a", "1", 1)], "1", None);
    bare["rows"][0].as_object_mut().unwrap().remove("object");
    api.reply(WIDGETS, 200, bare.clone());
    let options = TableOptions::default().include_object(IncludeObject::None);
    let mut feed = open(&api, &widget_gvk(), options).await;
    let first = next_ok(&mut feed).await;
    assert!(first.rows.contains_restart());
    let second = next_ok(&mut feed).await;
    assert!(
        second.rows.contains_restart(),
        "unkeyed rows cannot be diffed"
    );
    assert!(second.columns.is_some());
    assert_eq!(watches(&api, WIDGETS), 0);
}

#[tokio::test(start_paused = true)]
async fn dropping_the_feed_stops_its_task() {
    let api = server();
    api.reply(GADGETS, 200, widget_list(&[("a", "1", 1)], "1", None));
    let mut feed = open(&api, &gadget_gvk(), TableOptions::default()).await;
    next_ok(&mut feed).await;
    drop(feed);
    tokio::time::sleep(Duration::from_secs(600)).await;
    assert_eq!(
        lists(&api, GADGETS),
        1,
        "no refresh after the consumer left"
    );
}
