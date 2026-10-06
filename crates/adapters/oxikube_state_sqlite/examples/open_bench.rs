//! Open time of a typical state database (docs/PERFORMANCE.md, cold start).
//!
//! Builds a database the way a long-lived install looks (a layout, recent clusters, favourites
//! and a few thousand audit records), then times `SqliteState::open` (file open, `quick_check`,
//! migrations, on the worker thread) and the first layout read, over many cold opens.
//!
//! `cargo run -p oxikube_state_sqlite --release --example open_bench [audit_rows]`

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::time::{Duration, Instant};

use futures::executor::block_on;
use oxikube_domain::audit::{AuditOutcome, AuditRecord, Initiator};
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_ports::{StateKey, StatePort, StateTable};
use oxikube_state_sqlite::SqliteState;
use serde_json::json;

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    sorted[((sorted.len() - 1) as f64 * p).round() as usize]
}

fn main() {
    let audit_rows: usize = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(5_000);
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("state.db");

    block_on(async {
        let state = SqliteState::open(&path).await.expect("opens");
        let layout = json!({ "version": 1, "dock_area": { "center": { "items": (0..12).map(|i| json!({"kind": "pods", "state": {"ns": format!("ns-{i}")}})).collect::<Vec<_>>() } } });
        state
            .table_put(
                &StateTable::new("workspace_layout").unwrap(),
                &StateKey::new("main").unwrap(),
                layout,
            )
            .await
            .unwrap();
        for i in 0..200 {
            state
                .kv_set(
                    &StateKey::new(format!("recent/{i}")).unwrap(),
                    json!({"i": i}),
                )
                .await
                .unwrap();
        }
        let cluster = ClusterId::new("kubeconfig", &ContextName::new("prod"));
        let records: Vec<AuditRecord> = (0..audit_rows)
            .map(|i| {
                let target = ResourceRef::namespaced(
                    cluster.clone(),
                    Gvk::from_api_version("v1", "Pod"),
                    "default",
                    format!("web-{i}"),
                );
                AuditRecord::new(
                    jiff::Timestamp::from_second(1_790_000_000 + i as i64).unwrap(),
                    "alice",
                    Initiator::Ui,
                    "pod::Delete",
                    target,
                    false,
                    AuditOutcome::Succeeded,
                )
            })
            .collect();
        state.append_audit(&records).await.unwrap();
    });
    let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);

    let runs = 50;
    let (mut opens, mut reads) = (Vec::new(), Vec::new());
    for _ in 0..runs {
        let started = Instant::now();
        let state = block_on(SqliteState::open(&path)).expect("opens");
        opens.push(started.elapsed());
        let started = Instant::now();
        let layout = block_on(state.table_get(
            &StateTable::new("workspace_layout").unwrap(),
            &StateKey::new("main").unwrap(),
        ))
        .unwrap();
        reads.push(started.elapsed());
        assert!(layout.is_some());
    }
    opens.sort();
    reads.sort();
    println!(
        "db {} KiB, {audit_rows} audit rows, {runs} cold opens",
        size / 1024
    );
    for (name, v) in [("open", &opens), ("first layout read", &reads)] {
        println!(
            "{name:>18}: p50 {:>7.2?}  p95 {:>7.2?}  max {:>7.2?}",
            percentile(v, 0.5),
            percentile(v, 0.95),
            v.last().unwrap()
        );
    }
}
