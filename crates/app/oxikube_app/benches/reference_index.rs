//! Micro-benchmark (E12-S18): the reference index (`ReferenceIndex`, the "Used by" tab and the claims'
//! Pods column) over 10 000 Pods that each name a ServiceAccount, the cluster CA ConfigMap, a
//! Secret, an `env` ConfigMap and (a third of them) a claim.
//!
//! Prints min / median / p95 for what the story's performance note asks:
//!
//! * the first snapshot of 10 000 Pods (one pass) and the heap the index holds afterwards;
//! * a 500-op churn batch the way a busy cluster sends it: 450 status updates that name the
//!   same objects (no write lock, no index change) and 25 inserts and 25 removes that do;
//! * the per-delta cost of one Pod status update, and of one that changes a reference;
//! * the lookup a "Used by" tab pays (users of an object) and the claims table pays per visible
//!   row per frame (the Pods cell).
//!
//! `cargo bench -p oxikube_app --bench reference_index`. Under `cargo test --all-targets` (no
//! `--bench` flag) it runs one small iteration as a smoke test.

#![allow(clippy::print_stdout, reason = "a benchmark reports on stdout")]

use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use jiff::Timestamp;
use oxikube_app::columns::{ColumnId, ColumnProvider, CoreColumns};
use oxikube_app::store::{FeedState, RowChange, RowOp, StoreDelta, StoreObject};
use oxikube_app::{PODS_COLUMN, PvcColumns, ReferenceIndex};
use oxikube_domain::Resource;
use oxikube_domain::view::RefKind;
use oxikube_testkit::storage::pvc;
use serde_json::json;

const PODS: usize = 10_000;
const CHURN: usize = 500;

/// Counts the bytes the process holds, to report what the index costs.
struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);

// SAFETY: forwards to the system allocator and only counts.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        LIVE.fetch_add(layout.size(), Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

fn pod(i: usize, rv: usize, extra_ref: bool) -> Resource {
    let mut volumes = vec![
        json!({"name": "ca", "projected": {"sources": [{"configMap": {"name": "kube-root-ca.crt"}}]}}),
        json!({"name": "tls", "secret": {"secretName": format!("tls-{}", i % 50)}}),
    ];
    if i.is_multiple_of(3) {
        volumes.push(json!({"name": "d", "persistentVolumeClaim": {"claimName": format!("data-{}", i % 400)}}));
    }
    if extra_ref {
        volumes.push(json!({"name": "x", "configMap": {"name": "extra"}}));
    }
    let json = json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": {
            "name": format!("pod-{i:05}"), "namespace": format!("ns-{}", i % 20),
            "uid": format!("uid-{i}"), "resourceVersion": rv.to_string(),
            "ownerReferences": [{"apiVersion": "apps/v1", "kind": "ReplicaSet",
                "name": format!("rs-{}", i % 300), "uid": "u", "controller": true}],
        },
        "spec": {
            "serviceAccountName": "default",
            "volumes": volumes,
            "containers": [{"name": "c", "envFrom": [{"configMapRef": {"name": format!("env-{}", i % 100)}}]}],
        },
        "status": {"phase": "Running"},
    });
    Resource::from_json(json).expect("a pod")
}

fn pod_row(i: usize, rv: usize) -> Arc<StoreObject> {
    Arc::new(StoreObject::Resource(pod(i, rv, false)))
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
    println!("ReferenceIndex over {PODS} pods ({samples} samples)");

    let snapshot = delta(RowChange::Snapshot(rows));
    {
        let before = LIVE.load(Ordering::Relaxed);
        let index = ReferenceIndex::new();
        index.apply(&snapshot);
        let held = LIVE.load(Ordering::Relaxed).saturating_sub(before);
        println!(
            "heap held by the index for {PODS} pods: {:.1} MB ({} B per pod, {} indexed)",
            held as f64 / 1_048_576.0,
            held / PODS,
            index.indexed()
        );
    }
    report(
        "first snapshot (10k pods)",
        (0..samples)
            .map(|_| {
                let index = ReferenceIndex::new();
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
    let index = ReferenceIndex::new();
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

    // A pod whose references change (a new ConfigMap volume): the rare case, per pod.
    let changing: Vec<StoreDelta> = (0..samples * 20)
        .map(|k| {
            let i = (k * 37) % PODS;
            let extra = Arc::new(StoreObject::Resource(pod(i, k + 100_000, k % 2 == 0)));
            delta(RowChange::Ops(vec![RowOp::Update {
                index: i,
                object: extra,
            }]))
        })
        .collect();
    report(
        "one pod whose references changed",
        changing
            .iter()
            .map(|d| {
                let start = Instant::now();
                index.apply(d);
                start.elapsed()
            })
            .collect(),
    );

    // The cell a visible claims row pays per frame.
    let references = Arc::new({
        let index = ReferenceIndex::new();
        index.apply(&snapshot);
        index
    });
    let provider = PvcColumns::new(Arc::new(CoreColumns::new()), references.clone());
    let claim = StoreObject::Resource(pvc("data-3").namespace("ns-3").build());
    report(
        "users of a ConfigMap (kube-root-ca.crt, ns-3)",
        (0..samples * 20)
            .map(|_| {
                let start = Instant::now();
                black_box(references.usage(RefKind::ConfigMap, Some("ns-3"), "kube-root-ca.crt"));
                start.elapsed()
            })
            .collect(),
    );
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
