//! Micro-benchmark (E12-S15): the claims' "used by" index (`PvcUsageIndex`) over 10 000 Pods.
//!
//! Prints min / median / p95 for what the story's performance note asks:
//!
//! * the first snapshot of 10 000 Pods (one pass; a third of them mount a claim);
//! * a 500-op churn batch the way a busy cluster sends it: 450 status updates that mount the
//!   same claims (no write lock, no index change) and 25 inserts and 25 removes that do;
//! * the per-cell lookup the claims table pays per visible row per frame (`Pods` column), and the
//!   per-delta cost of one Pod update.
//!
//! `cargo bench -p oxikube_app --bench pvc_usage`. Under `cargo test --all-targets` (no `--bench`
//! flag) it runs one small iteration as a smoke test.

#![allow(clippy::print_stdout, reason = "a benchmark reports on stdout")]

use std::hint::black_box;
use std::sync::Arc;
use std::time::{Duration, Instant};

use jiff::Timestamp;
use oxikube_app::columns::{ColumnId, ColumnProvider, CoreColumns};
use oxikube_app::store::{FeedState, RowChange, RowOp, StoreDelta, StoreObject};
use oxikube_app::{PODS_COLUMN, PvcColumns, PvcUsage, PvcUsageIndex};
use oxikube_testkit::storage::{pod_mounting, pvc};

const PODS: usize = 10_000;
const CHURN: usize = 500;

fn pod_row(i: usize, rv: usize) -> Arc<StoreObject> {
    let claims: Vec<String> = if i.is_multiple_of(3) {
        vec![format!("data-{}", i % 400)]
    } else {
        Vec::new()
    };
    let refs: Vec<&str> = claims.iter().map(String::as_str).collect();
    let mut res = pod_mounting(&format!("pod-{i:05}"), &format!("ns-{}", i % 20), &refs);
    res.meta.resource_version = Some(rv.to_string().into());
    Arc::new(StoreObject::Resource(res))
}

fn delta(rows: RowChange) -> StoreDelta {
    StoreDelta {
        rows,
        state: FeedState::Ready,
        columns: None,
        len: PODS,
        total: PODS,
    }
}

fn report(label: &str, mut samples: Vec<Duration>) {
    samples.sort();
    let at = |q: f64| samples[((samples.len() - 1) as f64 * q) as usize];
    println!(
        "{label:<46} min {:>9.1?}  median {:>9.1?}  p95 {:>9.1?}",
        at(0.0),
        at(0.5),
        at(0.95)
    );
}

fn main() {
    let smoke = !std::env::args().any(|a| a == "--bench");
    let samples = if smoke { 2 } else { 60 };
    let rows: Vec<Arc<StoreObject>> = (0..PODS).map(|i| pod_row(i, 1)).collect();
    println!("PvcUsageIndex over {PODS} pods ({samples} samples)");

    let snapshot = delta(RowChange::Snapshot(rows));
    report(
        "first snapshot (10k pods)",
        (0..samples)
            .map(|_| {
                let index = PvcUsageIndex::new();
                let start = Instant::now();
                index.apply(&snapshot);
                start.elapsed()
            })
            .collect(),
    );

    // 450 updates of pods that mount the same claims, 25 removes, 25 inserts.
    let churn_ops = |round: usize| -> StoreDelta {
        let mut ops = Vec::with_capacity(CHURN);
        for k in 0..450 {
            let i = (round * 7_919 + k * 17) % PODS;
            ops.push(RowOp::Update {
                index: i,
                object: pod_row(i, round + 2),
            });
        }
        // Removes then inserts at the end keep every index valid.
        for k in 0..25 {
            ops.push(RowOp::Remove {
                index: PODS - 1 - k,
            });
        }
        for k in 0..25 {
            ops.push(RowOp::Insert {
                index: PODS - 25 + k,
                object: pod_row(PODS - 25 + k, round + 2),
            });
        }
        delta(RowChange::Ops(ops))
    };
    let index = PvcUsageIndex::new();
    index.apply(&snapshot);
    let batches: Vec<StoreDelta> = (0..samples).map(churn_ops).collect();
    report(
        "churn batch (450 updates, 25 adds, 25 removes)",
        batches
            .iter()
            .map(|batch| {
                let start = Instant::now();
                index.apply(batch);
                start.elapsed()
            })
            .collect(),
    );
    assert_eq!(index.pods(), PODS);

    // One status update that mounts the same claims: the common case, per pod.
    let single: Vec<StoreDelta> = (0..samples * 20)
        .map(|k| {
            let i = (k * 31) % PODS;
            delta(RowChange::Ops(vec![RowOp::Update {
                index: i,
                object: pod_row(i, k + 100),
            }]))
        })
        .collect();
    report(
        "one pod status update (same claims)",
        single
            .iter()
            .map(|d| {
                let start = Instant::now();
                index.apply(d);
                start.elapsed()
            })
            .collect(),
    );

    // The cell a visible claims row pays per frame.
    let usage: Arc<dyn PvcUsage> = Arc::new({
        let index = PvcUsageIndex::new();
        index.apply(&snapshot);
        index
    });
    let provider = PvcColumns::new(Arc::new(CoreColumns::new()), usage);
    let claim = StoreObject::Resource(pvc("data-3").namespace("ns-3").build());
    let column = ColumnId::new(PODS_COLUMN);
    let now: Timestamp = "2026-01-02T03:04:05Z".parse().unwrap();
    const BATCH: usize = 1_000;
    report(
        "Pods cell (per call)",
        (0..samples * 20)
            .map(|_| {
                let start = Instant::now();
                for _ in 0..BATCH {
                    black_box(provider.cell(black_box(&claim), &column, now));
                }
                start.elapsed() / BATCH as u32
            })
            .collect(),
    );
}
