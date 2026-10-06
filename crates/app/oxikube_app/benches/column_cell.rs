//! Micro-benchmark (E07-S02): the cost of `ColumnProvider::cell` and `columns`, which the table
//! pays per visible row per frame (E07-S09 budgets from these numbers).
//!
//! `cargo bench -p oxikube_app --bench column_cell` prints min / median / p95 per call for the
//! cheap path (name, JSON-pointer text), the computed pod columns (`ready`, `status`,
//! `restarts`, each of which builds a `PodSummary`), a whole default pod row, and the memoised
//! `columns()` header lookup. Under `cargo test --all-targets` (no `--bench` flag) it runs one
//! small iteration as a smoke test.

#![allow(clippy::print_stdout, reason = "a benchmark reports on stdout")]

use std::hint::black_box;
use std::time::{Duration, Instant};

use jiff::Timestamp;
use oxikube_app::columns::{ColumnId, ColumnProvider, CoreColumns};
use oxikube_app::store::StoreObject;
use oxikube_domain::Capabilities;
use oxikube_testkit::{fixtures, pod};

/// Calls per timed sample, so the clock resolution does not dominate.
const BATCH: usize = 1_000;

fn bench(label: &str, samples: usize, mut call: impl FnMut()) {
    for _ in 0..samples.min(20) {
        for _ in 0..BATCH {
            call();
        }
    }
    let mut per_call: Vec<Duration> = (0..samples)
        .map(|_| {
            let start = Instant::now();
            for _ in 0..BATCH {
                call();
            }
            start.elapsed() / u32::try_from(BATCH).unwrap_or(u32::MAX)
        })
        .collect();
    per_call.sort();
    let at = |q: f64| per_call[((per_call.len() - 1) as f64 * q) as usize];
    println!(
        "{label:<34} min {:>8.0?}  median {:>8.0?}  p95 {:>8.0?}",
        at(0.0),
        at(0.5),
        at(0.95)
    );
}

fn main() {
    let smoke = !std::env::args().any(|a| a == "--bench");
    let samples = if smoke { 3 } else { 200 };
    let now: Timestamp = "2026-01-02T03:04:05Z".parse().unwrap();
    let provider = CoreColumns::new();

    let pod_obj = StoreObject::Resource(pod().restarts(3).node("worker-1").ip("10.0.0.1").build());
    let deploy = StoreObject::Resource(fixtures::deployment_ready());
    let kind = match &pod_obj {
        StoreObject::Resource(r) => r.kind.clone(),
        StoreObject::Row(_) => unreachable!(),
    };
    let ids =
        |names: &[&str]| -> Vec<ColumnId> { names.iter().map(|n| ColumnId::new(*n)).collect() };
    let row_ids = ids(&[
        "name",
        "namespace",
        "ready",
        "status",
        "restarts",
        "node",
        "ip",
        "age",
    ]);

    println!("ColumnProvider cost per call ({samples} samples of {BATCH} calls)");
    let name = ColumnId::new("name");
    bench("cell: pod name", samples, || {
        black_box(provider.cell(black_box(&pod_obj), &name, now));
    });
    let node = ColumnId::new("node");
    bench("cell: pod node (JSON pointer)", samples, || {
        black_box(provider.cell(black_box(&pod_obj), &node, now));
    });
    let age = ColumnId::new("age");
    bench("cell: pod age", samples, || {
        black_box(provider.cell(black_box(&pod_obj), &age, now));
    });
    for id in ["ready", "status", "restarts"] {
        let id = ColumnId::new(id);
        bench(&format!("cell: pod {id} (PodSummary)"), samples, || {
            black_box(provider.cell(black_box(&pod_obj), &id, now));
        });
    }
    let ready = ColumnId::new("ready");
    bench("cell: deployment ready", samples, || {
        black_box(provider.cell(black_box(&deploy), &ready, now));
    });
    bench("row: 8 default pod columns", samples, || {
        for id in &row_ids {
            black_box(provider.cell(black_box(&pod_obj), id, now));
        }
    });
    bench("columns(): memoised header", samples, || {
        black_box(provider.columns(black_box(&kind), Capabilities::METRICS));
    });
}
