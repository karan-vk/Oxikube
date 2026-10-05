//! Hot-path report for the PR (docs/PERFORMANCE.md): a 10 000-row Table through the feed,
//! first load and a polled refresh with a handful of changes. The refresh must diff, so the
//! consumer gets one batch holding only the changed rows.
//! Run: `cargo test -p oxikube_kube --release --lib -- --ignored --nocapture bench_table`.

use std::time::{Duration, Instant};

use futures::StreamExt;
use oxikube_ports::{Delta, TableFeedPort, TableOptions};
use serde_json::Value;

use super::harness::*;
use crate::table::TableConfig;

const ROWS: usize = 10_000;
const PAGE: usize = 500;
const CHANGED: usize = 10;

/// The pages of one list of [`ROWS`] widgets; rows below `changed` carry version `2`.
fn pages(changed: usize, list_rv: &str) -> Vec<Value> {
    let names: Vec<String> = (0..ROWS).map(|i| format!("w-{i:05}")).collect();
    names
        .chunks(PAGE)
        .enumerate()
        .map(|(n, chunk)| {
            let rows: Vec<(&str, &str, i64)> = chunk
                .iter()
                .enumerate()
                .map(|(i, name)| {
                    let rv = if n * PAGE + i < changed { "2" } else { "1" };
                    (name.as_str(), rv, 1)
                })
                .collect();
            let more = (n + 1) * PAGE < ROWS;
            widget_list(&rows, list_rv, more.then_some("next"))
        })
        .collect()
}

#[tokio::test(start_paused = true)]
#[ignore = "timing report, not an assertion"]
async fn bench_table_feed_10k_rows_refresh_diffs() {
    let api = server();
    let first = pages(0, "100");
    let refresh = pages(CHANGED, "200");
    let first_bytes: usize = first.iter().map(|p| p.to_string().len()).sum();
    let refresh_bytes: usize = refresh.iter().map(|p| p.to_string().len()).sum();
    for page in first.into_iter().chain(refresh) {
        api.reply(GADGETS, 200, page);
    }
    let resources = adapter(&api, TableConfig::default());

    let started = Instant::now();
    let mut feed = resources
        .table_feed(&gadget_gvk(), Some(NS), &TableOptions::default())
        .await
        .expect("open feed");
    let batch = feed.next().await.expect("first").expect("first ok");
    let load = started.elapsed();
    let Some(Delta::Restarted(rows)) = batch.rows.deltas.first() else {
        panic!("first batch restarts");
    };
    assert_eq!(rows.len(), ROWS);

    // Paused time: the refresh interval elapses as soon as the feed task is idle.
    let started = Instant::now();
    let batch = tokio::time::timeout(Duration::from_secs(120), feed.next())
        .await
        .expect("refresh within the interval")
        .expect("refresh")
        .expect("refresh ok");
    let diff = started.elapsed();
    assert_eq!(batch.columns, None, "a refresh is a diff, not a restart");
    assert_eq!(batch.rows.len(), CHANGED, "only changed rows are sent");

    let rate = |d: Duration| ROWS as f64 / d.as_secs_f64();
    eprintln!(
        "table feed, {ROWS} rows in {} pages of {PAGE}:\n  \
         first load: {load:?} ({:.0} rows/s, {} KiB, 1 batch)\n  \
         refresh with {CHANGED} changed rows: {diff:?} ({:.0} rows/s, {} KiB, 1 batch of {} deltas)",
        ROWS / PAGE,
        rate(load),
        first_bytes / 1024,
        rate(diff),
        refresh_bytes / 1024,
        batch.rows.len(),
    );
}
