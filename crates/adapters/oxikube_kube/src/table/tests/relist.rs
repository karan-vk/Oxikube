//! Multi-page re-lists that go wrong part-way: a continue token that expires after the
//! first page restarts the list from page one (up to `max_restarts`), and printer columns
//! that change between two pages of one list are a retryable conflict the feed recovers
//! from with a fresh list.

use oxikube_domain::ErrorKind;
use oxikube_ports::{ListOptions, TableFeedPort, TableOptions};
use serde_json::Value;

use super::feed::{next, next_ok, open, settle, summary};
use super::harness::*;
use crate::fake_api::status_body;
use crate::is_list_expired;
use crate::table::TableConfig;

fn expired() -> Value {
    status_body(410, "Expired", "continue too old")
}

/// A widget list page whose columns lack the last definition (a CRD update).
fn narrower_list(rows: &[(&str, &str, i64)], rv: &str, continue_token: Option<&str>) -> Value {
    let mut page = widget_list(rows, rv, continue_token);
    page["columnDefinitions"].as_array_mut().unwrap().pop();
    page
}

#[tokio::test(start_paused = true)]
async fn an_expired_continue_token_restarts_the_relist_from_page_one() {
    let api = server();
    api.reply(
        WIDGETS,
        200,
        widget_list(&[("a", "1", 1), ("b", "1", 1), ("c", "1", 1)], "100", None),
    );
    api.reply_watch(WIDGETS, 200, &[error_event(410, "Expired")]);
    // The re-list: page one, then the token expires on page two ...
    api.reply(
        WIDGETS,
        200,
        widget_list(&[("a", "1", 1), ("b", "2", 5)], "200", Some("t1")),
    );
    api.reply(WIDGETS, 410, expired());
    // ... so it starts over and completes.
    api.reply(
        WIDGETS,
        200,
        widget_list(&[("a", "1", 1), ("b", "2", 5)], "200", Some("t2")),
    );
    api.reply(WIDGETS, 200, widget_list(&[("d", "1", 1)], "200", None));
    let options = TableOptions::default().list(ListOptions::default().limit(2));
    let mut feed = open(&api, &widget_gvk(), options).await;
    next_ok(&mut feed).await;

    // One diff batch for the whole recovered list: no error item, no partial table.
    let batch = next_ok(&mut feed).await;
    assert_eq!(
        summary(&batch),
        ["applied b@2", "applied d@1", "deleted c@1"]
    );
    assert_eq!(batch.columns, None, "same columns: a diff, not a restart");
    assert_eq!(batch.rows.resource_version.as_deref(), Some("200"));

    settle(|| watches(&api, WIDGETS) == 2).await;
    assert_eq!(
        lists(&api, WIDGETS),
        5,
        "first list + 2 re-list pages + 2 restart pages"
    );
    assert_eq!(
        query_of(&api, WIDGETS, false, 1),
        "limit=2&includeObject=Metadata"
    );
    assert_eq!(
        query_of(&api, WIDGETS, false, 2),
        "limit=2&continue=t1&includeObject=Metadata"
    );
    assert_eq!(
        query_of(&api, WIDGETS, false, 3),
        "limit=2&includeObject=Metadata",
        "the restart drops the expired token"
    );
    assert_eq!(
        query_of(&api, WIDGETS, false, 4),
        "limit=2&continue=t2&includeObject=Metadata"
    );
    assert!(query_of(&api, WIDGETS, true, 1).contains("resourceVersion=200"));
}

#[tokio::test(start_paused = true)]
async fn continue_token_restarts_are_capped() {
    let api = server();
    // max_restarts (3) + the first attempt: four page ones, each followed by a 410.
    for _ in 0..4 {
        api.reply(
            WIDGETS,
            200,
            widget_list(&[("a", "1", 1)], "100", Some("t")),
        );
        api.reply(WIDGETS, 410, expired());
    }
    let err = adapter(&api, TableConfig::default())
        .table_feed(&widget_gvk(), Some(NS), &TableOptions::default())
        .await
        .err()
        .expect("the token keeps expiring");
    assert!(is_list_expired(&err), "{err:?}");
    assert_eq!(lists(&api, WIDGETS), 8);
}

#[tokio::test(start_paused = true)]
async fn columns_changing_between_relist_pages_are_reported_then_recovered() {
    let api = server();
    api.reply(WIDGETS, 200, widget_list(&[("a", "1", 1)], "100", None));
    api.reply_watch(WIDGETS, 200, &[error_event(410, "Expired")]);
    // The re-list's first page has the sent columns; the CRD changes before page two.
    api.reply(
        WIDGETS,
        200,
        widget_list(&[("a", "1", 1)], "200", Some("t1")),
    );
    api.reply(WIDGETS, 200, narrower_list(&[("b", "1", 1)], "200", None));
    // After the retry delay the list is consistent again.
    api.reply(
        WIDGETS,
        200,
        narrower_list(&[("a", "2", 1), ("b", "1", 1)], "300", None),
    );
    let mut feed = open(&api, &widget_gvk(), TableOptions::default()).await;
    next_ok(&mut feed).await;

    let err = next(&mut feed).await.unwrap().unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Conflict);
    assert!(err.is_retryable());
    assert!(!is_list_expired(&err));

    let batch = next_ok(&mut feed).await;
    assert_eq!(summary(&batch), ["restarted [a@2 b@1]"]);
    assert_eq!(batch.columns.expect("the new columns").len(), 5);
    assert_eq!(lists(&api, WIDGETS), 4);
}

#[tokio::test(start_paused = true)]
async fn columns_changing_between_first_list_pages_fail_the_open() {
    let api = server();
    api.reply(
        WIDGETS,
        200,
        widget_list(&[("a", "1", 1)], "100", Some("t1")),
    );
    api.reply(WIDGETS, 200, narrower_list(&[("b", "1", 1)], "100", None));
    let err = adapter(&api, TableConfig::default())
        .table_feed(&widget_gvk(), Some(NS), &TableOptions::default())
        .await
        .err()
        .expect("mixed columns in one list");
    assert_eq!(err.kind(), ErrorKind::Conflict);
    assert!(err.is_retryable());
    assert_eq!(lists(&api, WIDGETS), 2);
}
