//! Micro-benchmark (E13-S02): the cost of the utilisation maths the metrics poll calls for the
//! visible rows (E13-S03) and of whole-cluster allocation (E13-S11).
//!
//! `cargo bench -p oxikube_app --bench utilisation` prints min / median / p95 for one pod's
//! requests and limits, one pod's four percents, and the allocation of 10 000 pods over 100
//! nodes and 50 namespaces. Under `cargo test --all-targets` (no `--bench` flag) it runs one
//! small iteration as a smoke test.
//!
//! It also times what the allocation panels (E13-S11) do when their pods change: a node's panel
//! (110 pods: sums plus view) and a namespace's (2 000 pods: sums plus view, with a quota).

#![allow(clippy::print_stdout, reason = "a benchmark reports on stdout")]

use std::hint::black_box;
use std::time::{Duration, Instant};

use oxikube_app::allocation::{AllocationView, UsageState};
use oxikube_app::{UtilisationAggregator, allocate_namespaces, allocate_nodes};
use oxikube_domain::Resource;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_domain::utilisation::{
    GpuVendors, QuotaCaps, Utilisation, aggregate_pod_requests_limits,
};
use serde_json::json;

const PODS: usize = 10_000;
const NODES: usize = 100;
const NAMESPACES: usize = 50;

fn pod(i: usize) -> Resource {
    pod_at(
        i,
        &format!("ns-{}", i % NAMESPACES),
        &format!("node-{}", i % NODES),
    )
}

fn pod_at(i: usize, namespace: &str, node: &str) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": {"name": format!("pod-{i:05}"), "namespace": namespace},
        "spec": {
            "nodeName": node,
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

    // What a panel redoes when one of its pods changes.
    let on_node: Vec<Resource> = (0..110).map(|i| pod_at(i, "ns-0", "node-0")).collect();
    time("node panel: 110 pods, sums + view", samples, 100, || {
        let all = allocate_nodes(&nodes[..1], &on_node, &gpus);
        black_box(AllocationView::for_node(&all[0], &UsageState::Pending));
    });
    let in_namespace: Vec<Resource> = (0..2000).map(|i| pod_at(i, "ns-0", "node-0")).collect();
    let quota = Resource::from_json(json!({
        "apiVersion": "v1", "kind": "ResourceQuota",
        "metadata": {"name": "q", "namespace": "ns-0"},
        "spec": {"hard": {"requests.cpu": "1000", "limits.cpu": "2000", "pods": "5000"}},
    }))
    .unwrap();
    time("namespace panel: 2k pods, sums + view", samples, 1, || {
        let all = allocate_namespaces(&in_namespace, &gpus);
        let (caps, scoped) = QuotaCaps::tightest([&quota], &gpus);
        black_box(AllocationView::for_namespace(
            &all[0],
            &caps,
            scoped,
            &UsageState::Pending,
        ));
    });
}
