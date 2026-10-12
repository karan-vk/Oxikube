//! Micro-benchmark (E13-S03): what a metrics poll and a usage cell cost.
//!
//! `cargo bench -p oxikube_app --bench live_metrics` prints min / median / p95 for
//!
//! * one poll of 10 000 pods landing in `ClusterUsage::apply` (on the poller's Tokio task, never
//!   the UI thread): the batch the column cells read afterwards;
//! * one usage cell (`CPU`) and one `%MEM/L` cell of a pod row through `ClusterPodColumns`, which
//!   the table pays per visible row per redraw;
//!
//! and the size of the 60-sample rings of 10 000 pods (the memory budget's share). Under
//! `cargo test --all-targets` (no `--bench` flag) it runs one small iteration as a smoke test.

#![allow(clippy::print_stdout, reason = "a benchmark reports on stdout")]

use std::hint::black_box;
use std::sync::Arc;
use std::time::{Duration, Instant};

use jiff::{SignedDuration, Timestamp};
use oxikube_app::columns::{ColumnId, ColumnProvider, PodColumns};
use oxikube_app::metrics::live::{ClusterUsage, MetricsKind, RING_CAPACITY, SampleRing};
use oxikube_app::store::StoreObject;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_domain::metrics::{MetricsSample, MetricsSubject};
use oxikube_ports::MetricsOutcome;
use oxikube_testkit::pod;

fn bench(label: &str, samples: usize, batch: usize, mut call: impl FnMut()) {
    let mut per_call: Vec<Duration> = (0..samples)
        .map(|_| {
            let start = Instant::now();
            for _ in 0..batch {
                call();
            }
            start.elapsed() / u32::try_from(batch).unwrap_or(u32::MAX)
        })
        .collect();
    per_call.sort();
    let at = |q: f64| per_call[((per_call.len() - 1) as f64 * q) as usize];
    println!(
        "{label:<40} min {:>9.1?}  median {:>9.1?}  p95 {:>9.1?}",
        at(0.0),
        at(0.5),
        at(0.95)
    );
}

fn poll(cluster: &ClusterId, pods: usize, at: Timestamp) -> MetricsOutcome {
    MetricsOutcome::Available(
        (0..pods)
            .map(|i| {
                let pod = ResourceRef::namespaced(
                    cluster.clone(),
                    Gvk::new("", "v1", "Pod"),
                    format!("ns-{}", i % 50),
                    format!("pod-{i}"),
                );
                MetricsSample::new(
                    MetricsSubject::Pod { pod },
                    at,
                    None,
                    (i as u64 % 900 + 1) * 1_000_000,
                    (i as u64 % 512 + 16) << 20,
                )
            })
            .collect(),
    )
}

fn main() {
    let smoke = !std::env::args().any(|a| a == "--bench");
    let (samples, pods) = if smoke { (2, 100) } else { (30, 10_000) };
    let cluster = ClusterId::new("/k", &ContextName::new("bench"));
    let usage = Arc::new(ClusterUsage::new());

    // A poll per 15 s: each lands with a new scrape time, so every ring takes a reading.
    let mut tick = 0;
    let polls: Vec<MetricsOutcome> = (0..samples + 1)
        .map(|i| {
            poll(
                &cluster,
                pods,
                Timestamp::UNIX_EPOCH + SignedDuration::from_secs(15 * i as i64),
            )
        })
        .collect();
    usage.apply(
        MetricsKind::Pods,
        Timestamp::UNIX_EPOCH,
        &Ok(polls[0].clone()),
    );
    bench(
        &format!("apply one poll of {pods} pods"),
        samples,
        1,
        || {
            tick += 1;
            usage.apply(
                MetricsKind::Pods,
                Timestamp::UNIX_EPOCH,
                &Ok(polls[tick].clone()),
            );
        },
    );

    let columns = Arc::new(PodColumns::new()).in_cluster(Some(usage));
    let row = StoreObject::Resource(
        pod()
            .uid("uid-bench")
            .resource_version("1")
            .namespace("ns-7")
            .name("pod-7")
            .limits("1", "1Gi")
            .build(),
    );
    let now = Timestamp::UNIX_EPOCH;
    let (cpu, mem_l) = (ColumnId::new("cpu"), ColumnId::new("memory-limit-pct"));
    let cell_samples = if smoke { 2 } else { 200 };
    bench("cell: CPU", cell_samples, 1_000, || {
        black_box(columns.cell(black_box(&row), &cpu, now));
    });
    bench("cell: %MEM/L", cell_samples, 1_000, || {
        black_box(columns.cell(black_box(&row), &mem_l, now));
    });

    let ring = std::mem::size_of::<SampleRing>() + RING_CAPACITY * 24;
    println!(
        "rings of {pods} pods: ~{:.1} MiB ({} B per pod)",
        (ring * pods) as f64 / (1 << 20) as f64,
        ring
    );
}
