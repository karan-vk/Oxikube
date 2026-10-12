//! Top consumers: the bounded heap agrees with a full sort, and a metrics poll gives the top pods
//! and namespaces for CPU and memory.

use std::sync::Arc;

use jiff::Timestamp;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_domain::metrics::{MetricsSample, MetricsSubject};
use oxikube_ports::MetricsOutcome;
use proptest::prelude::*;

use crate::cluster_overview::{TopEntry, top_consumers, top_n};
use crate::metrics::{ClusterUsage, MetricsKind};

const MILLI: u64 = 1_000_000;
const MI: u64 = 1024 * 1024;

fn entry(ns: &str, name: &str, value: u64) -> TopEntry {
    TopEntry {
        namespace: Some(ns.into()),
        name: name.into(),
        value,
    }
}

fn names(entries: &[TopEntry]) -> Vec<&str> {
    entries.iter().map(|e| &*e.name).collect()
}

#[test]
fn the_largest_come_first_and_ties_go_by_name() {
    let entries = vec![
        entry("a", "small", 1),
        entry("a", "big", 9),
        entry("a", "b-tie", 5),
        entry("a", "a-tie", 5),
        entry("a", "mid", 3),
    ];
    assert_eq!(names(&top_n(entries.clone(), 3)), ["big", "a-tie", "b-tie"]);
    assert_eq!(top_n(entries.clone(), 10).len(), 5);
    assert!(top_n(entries, 0).is_empty());
}

proptest! {
    #[test]
    fn the_bounded_heap_agrees_with_a_full_sort(values in prop::collection::vec(0u64..50, 0..200), n in 0usize..12) {
        let entries: Vec<TopEntry> = values
            .iter()
            .enumerate()
            .map(|(i, v)| entry("ns", &format!("p{i:03}"), *v))
            .collect();
        let mut sorted = entries.clone();
        sorted.sort_by(|a, b| b.value.cmp(&a.value).then_with(|| a.name.cmp(&b.name)));
        sorted.truncate(n);
        prop_assert_eq!(top_n(entries, n), sorted);
    }
}

fn pod_sample(ns: &str, name: &str, millis: u64, mib: u64) -> MetricsSample {
    let cluster = ClusterId::new("test", &ContextName::from("kind"));
    MetricsSample::new(
        MetricsSubject::Pod {
            pod: ResourceRef::namespaced(cluster, Gvk::new("", "v1", "Pod"), ns, name),
        },
        Timestamp::UNIX_EPOCH,
        None,
        millis * MILLI,
        mib * MI,
    )
}

#[test]
fn a_poll_ranks_pods_and_sums_namespaces() {
    let usage = Arc::new(ClusterUsage::new());
    usage.apply(
        MetricsKind::Pods,
        Timestamp::UNIX_EPOCH,
        &Ok(MetricsOutcome::Available(vec![
            pod_sample("shop", "web-1", 400, 100),
            pod_sample("shop", "web-2", 300, 600),
            pod_sample("data", "db-0", 500, 300),
            pod_sample("kube-system", "dns", 10, 20),
        ])),
    );
    let top = top_consumers(&usage, 2);
    assert_eq!(names(&top.pods_cpu), ["db-0", "web-1"]);
    assert_eq!(names(&top.pods_memory), ["web-2", "db-0"]);
    assert_eq!(top.pods_cpu[0].namespace.as_deref(), Some("data"));
    assert_eq!(names(&top.namespaces_cpu), ["shop", "data"]);
    assert_eq!(top.namespaces_cpu[0].value, 700 * MILLI);
    assert_eq!(top.namespaces_cpu[0].namespace, None);
    assert_eq!(names(&top.namespaces_memory), ["shop", "data"]);
    assert_eq!(top.namespaces_memory[0].value, 700 * MI);
}

#[test]
fn no_readings_no_rows() {
    let top = top_consumers(&ClusterUsage::new(), 5);
    assert!(top.pods_cpu.is_empty() && top.namespaces_memory.is_empty());
}
