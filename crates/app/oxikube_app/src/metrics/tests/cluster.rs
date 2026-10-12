//! Cluster-wide CPU and memory use (E13-S10): sums over the reporting nodes, the history built
//! from the per-node rings, and the cases that must read as "nothing", never as zero.

use jiff::{SignedDuration, Timestamp};
use oxikube_domain::metrics::{MetricsSample, MetricsSubject};
use oxikube_domain::utilisation::GpuVendors;
use oxikube_ports::MetricsOutcome;
use serde_json::json;

use super::node;
use crate::metrics::cluster_utilisation;
use crate::metrics::live::{ClusterUsage, MetricsKind};

const MILLI: u64 = 1_000_000;
const MI: u64 = 1024 * 1024;

fn t0() -> Timestamp {
    "2026-01-01T00:00:00Z".parse().expect("a timestamp")
}

fn at(seconds: i64) -> Timestamp {
    t0() + SignedDuration::from_secs(seconds)
}

fn sample(name: &str, ts: Timestamp, millis: u64, mib: u64) -> MetricsSample {
    MetricsSample::new(
        MetricsSubject::Node { node: name.into() },
        ts,
        None,
        millis * MILLI,
        mib * MI,
    )
}

fn poll(usage: &ClusterUsage, ts: Timestamp, samples: Vec<MetricsSample>) {
    usage.apply(
        MetricsKind::Nodes,
        ts,
        &Ok(MetricsOutcome::Available(samples)),
    );
}

#[test]
fn the_latest_readings_are_summed_over_the_nodes_allocatable() {
    let usage = ClusterUsage::new();
    poll(
        &usage,
        at(1),
        vec![
            sample("a", at(0), 1000, 2048),
            sample("b", at(0), 500, 1024),
        ],
    );
    let nodes = [
        node("a", json!({"cpu": "4", "memory": "8Gi"})),
        node("b", json!({"cpu": "4", "memory": "8Gi"})),
    ];
    let found = cluster_utilisation(&usage, &nodes, &GpuVendors::none()).expect("readings");
    assert_eq!((found.nodes, found.nodes_reporting), (2, 2));
    assert_eq!(found.cpu.used, 1500 * u128::from(MILLI));
    assert_eq!(found.cpu.allocatable, 8_000 * u128::from(MILLI));
    assert_eq!(found.cpu.percent, Some(18.75));
    assert_eq!(found.memory.used, 3072 * u128::from(MI));
    assert_eq!(found.memory.percent, Some(3072.0 * 100.0 / 16384.0));
}

#[test]
fn a_node_without_a_reading_counts_for_neither_side() {
    let usage = ClusterUsage::new();
    poll(&usage, at(1), vec![sample("a", at(0), 2000, 1024)]);
    let nodes = [
        node("a", json!({"cpu": "4", "memory": "4Gi"})),
        node("silent", json!({"cpu": "64", "memory": "256Gi"})),
    ];
    let found = cluster_utilisation(&usage, &nodes, &GpuVendors::none()).expect("readings");
    assert_eq!((found.nodes, found.nodes_reporting), (2, 1));
    assert_eq!(
        found.cpu.percent,
        Some(50.0),
        "2 of the one reporting node's 4 cores"
    );
}

#[test]
fn no_readings_is_none_not_zero() {
    let usage = ClusterUsage::new();
    let nodes = [node("a", json!({"cpu": "4", "memory": "4Gi"}))];
    assert_eq!(
        cluster_utilisation(&usage, &nodes, &GpuVendors::none()),
        None
    );
    assert_eq!(cluster_utilisation(&usage, [], &GpuVendors::none()), None);
}

#[test]
fn a_node_that_offers_nothing_has_no_percent() {
    let usage = ClusterUsage::new();
    poll(&usage, at(1), vec![sample("a", at(0), 100, 100)]);
    let nodes = [node("a", json!({}))];
    let found = cluster_utilisation(&usage, &nodes, &GpuVendors::none()).expect("readings");
    assert_eq!(found.cpu.percent, None);
    assert!(found.cpu.history.is_empty());
}

#[test]
fn the_history_adds_the_nodes_polls_oldest_first() {
    let usage = ClusterUsage::new();
    for (i, (a, b)) in [(1000, 1000), (2000, 1000), (3000, 1000)]
        .into_iter()
        .enumerate()
    {
        let ts = at(15 * i as i64);
        poll(
            &usage,
            ts,
            vec![sample("a", ts, a, 512), sample("b", ts, b, 512)],
        );
    }
    let nodes = [
        node("a", json!({"cpu": "4", "memory": "4Gi"})),
        node("b", json!({"cpu": "4", "memory": "4Gi"})),
    ];
    let found = cluster_utilisation(&usage, &nodes, &GpuVendors::none()).expect("readings");
    let percents: Vec<f64> = found.cpu.history.iter().map(|(_, p)| *p).collect();
    assert_eq!(percents, [25.0, 37.5, 50.0]);
    let times: Vec<Timestamp> = found.cpu.history.iter().map(|(t, _)| *t).collect();
    assert_eq!(times, [at(0), at(15), at(30)]);
    assert_eq!(
        found.cpu.percent,
        Some(50.0),
        "the newest point is the figure"
    );
}

#[test]
fn a_node_that_joined_later_does_not_dilute_the_older_history() {
    let usage = ClusterUsage::new();
    poll(&usage, at(1), vec![sample("a", at(0), 2000, 512)]);
    poll(
        &usage,
        at(16),
        vec![
            sample("a", at(15), 2000, 512),
            sample("b", at(15), 2000, 512),
        ],
    );
    let nodes = [
        node("a", json!({"cpu": "4", "memory": "4Gi"})),
        node("b", json!({"cpu": "4", "memory": "4Gi"})),
    ];
    let found = cluster_utilisation(&usage, &nodes, &GpuVendors::none()).expect("readings");
    let percents: Vec<f64> = found.cpu.history.iter().map(|(_, p)| *p).collect();
    // Before `b` existed the cluster was `a` alone: 2 of 4 cores, not 2 of 8.
    assert_eq!(percents, [50.0, 50.0]);
}
