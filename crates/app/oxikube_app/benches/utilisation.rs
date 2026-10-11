//! Micro-benchmark (E13-S02): the cost of the utilisation maths the metrics poll calls for the
//! visible rows (E13-S03) and of whole-cluster allocation (E13-S11).
//!
//! `cargo bench -p oxikube_app --bench utilisation` prints min / median / p95 for one pod's
//! requests and limits, one pod's four percents, and the allocation of 10 000 pods over 100
//! nodes and 50 namespaces. Under `cargo test --all-targets` (no `--bench` flag) it runs one
//! small iteration as a smoke test.

#![allow(clippy::print_stdout, reason = "a benchmark reports on stdout")]

use std::hint::black_box;
use std::time::{Duration, Instant};

use oxikube_app::{UtilisationAggregator, allocate_namespaces, allocate_nodes};
use oxikube_domain::Resource;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_domain::utilisation::{GpuVendors, Utilisation, aggregate_pod_requests_limits};
use serde_json::json;

const PODS: usize = 10_000;
const NODES: usize = 100;
const NAMESPACES: usize = 50;

fn pod(i: usize) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": {"name": format!("pod-{i:05}"), "namespace": format!("ns-{}", i % NAMESPACES)},
        "spec": {
            "nodeName": format!("node-{}", i % NODES),
            "initContainers": [{"name": "init", "resources": {"requests": {"cpu": "100m", "memory": "64Mi"}}}],
            "containers": [
                {"name": "app", "resources": {
                    "requests": {"cpu": "250m", "memory": "128Mi"},
                    "limits": {"cpu": "1", "memory": "512Mi"}}},
                {"name": "sidecar", "resources": {
                    "requests": {"cpu": "50m", "memory": "32Mi"},
                    "limits": {"memory": "64Mi"}}},
            ],
        },
        "status": {"phase": if i.is_multiple_of(20) { "Succeeded" } else { "Running" }},
    }))
    .unwrap()
}

fn node(i: usize) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1", "kind": "Node",
        "metadata": {"name": format!("node-{i}")},
        "status": {"allocatable": {"cpu": "32", "memory": "128Gi", "pods": "250"}},
    }))
    .unwrap()
}

fn report(label: &str, samples: &mut [Duration]) {
    samples.sort();
    let at = |q: f64| samples[((samples.len() - 1) as f64 * q) as usize];
    println!(
        "{label:<40} min {:>9.0?}  median {:>9.0?}  p95 {:>9.0?}",
        at(0.0),
        at(0.5),
        at(0.95)
    );
}

fn time(label: &str, samples: usize, batch: usize, mut call: impl FnMut()) {
    for _ in 0..samples.min(10) {
        call();
    }
    let mut per_call: Vec<Duration> = (0..samples)
        .map(|_| {
            let start = Instant::now();
            for _ in 0..batch {
                call();
            }
            start.elapsed() / u32::try_from(batch).unwrap_or(u32::MAX)
        })
        .collect();
    report(label, &mut per_call);
}

fn main() {
    let smoke = !std::env::args().any(|a| a == "--bench");
    let samples = if smoke { 3 } else { 100 };
    let pods: Vec<Resource> = (0..PODS).map(pod).collect();
    let nodes: Vec<Resource> = (0..NODES).map(node).collect();
    let gpus = GpuVendors::default();
    let cluster = ClusterId::new("bench", &ContextName::new("c"));
    let aggregator = UtilisationAggregator::default();

    println!("utilisation maths ({PODS} pods, {NODES} nodes, {NAMESPACES} namespaces)");
    time("pod: requests and limits", samples, 1000, || {
        black_box(aggregate_pod_requests_limits(black_box(&pods[1]), &gpus).unwrap());
    });
    let resources = aggregate_pod_requests_limits(&pods[1], &gpus).unwrap();
    let (cpu, mem) = (
        oxikube_domain::Quantity::from_milli(120),
        oxikube_domain::Quantity::parse("90Mi").unwrap(),
    );
    time("pod: four percents from usage", samples, 1000, || {
        black_box(Utilisation::of(
            Some(black_box(&cpu)),
            Some(black_box(&mem)),
            &resources,
        ));
    });
    time("pod: requests via the aggregator", samples, 1000, || {
        black_box(
            aggregator
                .pod_resources(&cluster, black_box(&pods[1]))
                .unwrap(),
        );
    });
    time("nodes: allocation of 10k pods", samples, 1, || {
        black_box(allocate_nodes(&nodes, &pods, &gpus));
    });
    time("namespaces: totals of 10k pods", samples, 1, || {
        black_box(allocate_namespaces(&pods, &gpus));
    });
}
