//! Micro-benchmark (E13-S09): what the cluster overview's feed pays, off the UI thread. A pod's
//! change (one tally upsert), a first list of 10 000 pods counted from scratch, 100 nodes and the
//! distribution rules over them, and the top consumers of a 10 000-pod metrics poll.
//!
//! `cargo bench -p oxikube_app --bench cluster_overview` prints min / median / p95. Under
//! `cargo test --all-targets` (no `--bench` flag) it runs a few iterations as a smoke test.

#![allow(clippy::print_stdout, reason = "a benchmark reports on stdout")]

use std::hint::black_box;
use std::time::{Duration, Instant};

use jiff::Timestamp;
use oxikube_app::cluster_overview::{
    NodeTally, PodTally, TOP_N, detect_distribution, top_consumers,
};
use oxikube_app::store::ObjectKey;
use oxikube_app::{ClusterUsage, MetricsKind};
use oxikube_domain::Resource;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_domain::metrics::{MetricsSample, MetricsSubject};
use oxikube_ports::MetricsOutcome;
use serde_json::json;

const PODS: usize = 10_000;
const NODES: usize = 100;
const NAMESPACES: usize = 50;

fn pod(i: usize, phase: &str) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": {"name": format!("pod-{i:05}"), "namespace": format!("ns-{}", i % NAMESPACES)},
        "spec": {
            "nodeName": format!("node-{}", i % NODES),
            "containers": [
                {"name": "app", "resources": {
                    "requests": {"cpu": "250m", "memory": "128Mi"},
                    "limits": {"cpu": "1", "memory": "512Mi"}}},
                {"name": "sidecar", "resources": {"requests": {"cpu": "50m", "memory": "32Mi"}}},
            ],
        },
        "status": {"phase": phase},
    }))
    .unwrap()
}

fn node(i: usize) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1", "kind": "Node",
        "metadata": {"name": format!("node-{i}"), "labels": {"kubernetes.io/os": "linux"}},
        "spec": {"providerID": format!("aws:///eu-west-1a/i-{i:08}")},
        "status": {
            "allocatable": {"cpu": "32", "memory": "128Gi", "pods": "250"},
            "conditions": [{"type": "Ready", "status": "True"}],
        },
    }))
    .unwrap()
}

fn time(label: &str, samples: usize, batch: usize, mut call: impl FnMut()) {
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
        "{label:<44} min {:>9.0?}  median {:>9.0?}  p95 {:>9.0?}",
        at(0.0),
        at(0.5),
        at(0.95)
    );
}

fn main() {
    let smoke = !std::env::args().any(|a| a == "--bench");
    let samples = if smoke { 3 } else { 50 };
    let running: Vec<Resource> = (0..PODS).map(|i| pod(i, "Running")).collect();
    let pending: Vec<Resource> = (0..PODS).map(|i| pod(i, "Pending")).collect();
    let nodes: Vec<Resource> = (0..NODES).map(node).collect();
    let cluster = ClusterId::new("bench", &ContextName::new("c"));

    println!("cluster overview ({PODS} pods, {NODES} nodes, {NAMESPACES} namespaces)");
    let mut tally = PodTally::of(running.iter());
    let mut i = 0;
    time("pod: one phase change (upsert)", samples, 1000, || {
        let p = if i % 2 == 0 {
            &pending[i % PODS]
        } else {
            &running[i % PODS]
        };
        tally.upsert(ObjectKey::of(&p.meta), black_box(p));
        i += 1;
    });
    time("pods: first list of 10k counted", samples, 1, || {
        black_box(PodTally::of(running.iter()));
    });
    time("nodes: 100 counted", samples, 10, || {
        black_box(NodeTally::of(nodes.iter()));
    });
    time("nodes: distribution rules over 100", samples, 100, || {
        black_box(detect_distribution(Some("v1.33.1"), nodes.iter()));
    });
    let usage = ClusterUsage::new();
    let samples_10k: Vec<MetricsSample> = (0..PODS)
        .map(|i| {
            let pod = ResourceRef::namespaced(
                cluster.clone(),
                Gvk::new("", "v1", "Pod"),
                format!("ns-{}", i % NAMESPACES),
                format!("pod-{i:05}"),
            );
            let cpu = (i as u64 * 7919) % 2_000 * 1_000_000;
            MetricsSample::new(
                MetricsSubject::Pod { pod },
                Timestamp::UNIX_EPOCH,
                None,
                cpu,
                cpu * 64,
            )
        })
        .collect();
    usage.apply(
        MetricsKind::Pods,
        Timestamp::UNIX_EPOCH,
        &Ok(MetricsOutcome::Available(samples_10k)),
    );
    time(
        "metrics: top consumers of a 10k-pod poll",
        samples,
        1,
        || {
            black_box(top_consumers(&usage, TOP_N));
        },
    );
}
