//! [`WorldMetrics`]: the synthetic cluster's metrics-server (E13-S09-F1).
//!
//! A synthetic cluster with metrics answers for the objects it holds: one reading per node
//! (`worker-<n>`) and one per pod (`load-<n>` in `oxikube-load-<n>`). A reading is a pure
//! function of the object's index and the clock, so the same pod reads about the same from poll
//! to poll (a slow drift aside) and two runs see the same numbers.
//!
//! The values are plausible for the fixture: a node has 16 cores and 64 GiB and uses 15 to 70 %
//! of them, a pod uses 20 to 1 000 millicores and 16 MiB to 1 GiB.
//!
//! Without this port the cluster's overview draws its `metrics unavailable` state (the
//! `overview` scenario); with it the usage bars, the top consumers and the per-node sparklines
//! have numbers to draw (the `overview-graphs` scenario).

use std::sync::Arc;

use async_trait::async_trait;
use jiff::Timestamp;
use oxikube_domain::OxiResult;
use oxikube_domain::ids::{ClusterId, Gvk};
use oxikube_domain::metrics::{MetricsSample, MetricsSubject};
use oxikube_domain::session::NamespaceSelection;
use oxikube_ports::{MetricsOutcome, MetricsPort};

use super::population::namespace_name;

/// The cores of a synthetic node.
const NODE_CORES: u64 = 16;
/// The bytes of a synthetic node's memory (64 GiB).
const NODE_BYTES: u64 = 64 * 1024 * 1024 * 1024;
/// Nanocores in one core.
const NANOCORES: u64 = 1_000_000_000;
/// Bytes in one mebibyte.
const MEBIBYTE: u64 = 1024 * 1024;

/// The `Pod` kind, the pods' kind in the synthetic world.
fn pods() -> Gvk {
    Gvk::new("", "v1", "Pod")
}

/// The name of the node at `index`.
fn node_name(index: usize) -> String {
    format!("worker-{index}")
}

/// The name of the pod at `index`.
fn pod_name(index: usize) -> String {
    format!("load-{index:05}")
}

/// The namespace of the pod at `index`.
fn pod_namespace(index: usize, namespaces: usize) -> String {
    namespace_name(index % namespaces.max(1))
}

/// The slow drift of a reading: a fraction of its value, moving with the clock. Small enough
/// that the numbers do not jump between two polls, large enough that they do move.
fn drift(index: usize, ts: Timestamp) -> f64 {
    let seconds = ts.as_second() as f64;
    let phase = index as f64 * 0.7;
    0.04 * (seconds / 45.0 + phase).sin()
}

/// The node at `index`: its CPU in nanocores and its memory in bytes.
fn node_reading(index: usize, ts: Timestamp) -> (u64, u64) {
    let cpu_fraction = 0.15 + 0.55 * f64::from((index * 37 % 100) as u32) / 100.0;
    let memory_fraction = 0.25 + 0.45 * f64::from((index * 53 % 100) as u32) / 100.0;
    let drift = drift(index, ts);
    let cpu = (NODE_CORES as f64 * NANOCORES as f64 * cpu_fraction * (1.0 + drift)) as u64;
    let memory = (NODE_BYTES as f64 * memory_fraction * (1.0 - drift)) as u64;
    (cpu, memory)
}

/// The pod at `index`: its CPU in nanocores and its memory in bytes.
fn pod_reading(index: usize, ts: Timestamp) -> (u64, u64) {
    let millicores = 20 + (index * 61 % 980) as u64;
    let mebibytes = 16 + (index * 29 % 1000) as u64;
    let drift = drift(index, ts);
    let cpu = (millicores as f64 * 1_000_000.0 * (1.0 + drift)) as u64;
    let memory = (mebibytes as f64 * MEBIBYTE as f64 * (1.0 - drift)) as u64;
    (cpu, memory)
}

/// The metrics-server of one synthetic cluster.
#[derive(Debug, Clone)]
pub struct WorldMetrics {
    /// The nodes it reports.
    nodes: usize,
    /// The pods it reports.
    pods: usize,
    /// The namespaces the pods are spread over.
    namespaces: usize,
}

impl WorldMetrics {
    /// The metrics-server of a cluster with `nodes` nodes and `pods` pods over `namespaces`
    /// namespaces.
    pub fn new(nodes: usize, pods: usize, namespaces: usize) -> Self {
        Self {
            nodes,
            pods,
            namespaces,
        }
    }
}

#[async_trait]
impl MetricsPort for WorldMetrics {
    async fn node_metrics(&self, _cluster: &ClusterId) -> OxiResult<MetricsOutcome> {
        let ts = Timestamp::now();
        let samples = (0..self.nodes)
            .map(|index| {
                let (cpu, memory) = node_reading(index, ts);
                MetricsSample::new(
                    MetricsSubject::Node {
                        node: Arc::from(node_name(index)),
                    },
                    ts,
                    None,
                    cpu,
                    memory,
                )
            })
            .collect();
        Ok(MetricsOutcome::Available(samples))
    }

    async fn pod_metrics(
        &self,
        cluster: &ClusterId,
        namespaces: &NamespaceSelection,
    ) -> OxiResult<MetricsOutcome> {
        let ts = Timestamp::now();
        let samples = (0..self.pods)
            .filter(|index| {
                let namespace = pod_namespace(*index, self.namespaces);
                namespaces.contains(&namespace)
            })
            .map(|index| {
                let (cpu, memory) = pod_reading(index, ts);
                MetricsSample::new(
                    MetricsSubject::Pod {
                        pod: oxikube_domain::ids::ResourceRef::namespaced(
                            cluster.clone(),
                            pods(),
                            pod_namespace(index, self.namespaces),
                            pod_name(index),
                        ),
                    },
                    ts,
                    None,
                    cpu,
                    memory,
                )
            })
            .collect();
        Ok(MetricsOutcome::Available(samples))
    }
}

#[cfg(test)]
mod tests {
    use futures::executor::block_on;

    use super::*;

    fn cluster() -> ClusterId {
        ClusterId::new(
            "perf-kubeconfig",
            &oxikube_domain::ids::ContextName::new("perf-a"),
        )
    }

    fn metrics() -> WorldMetrics {
        WorldMetrics::new(100, 3_000, 8)
    }

    #[test]
    fn every_node_gets_a_reading() {
        let outcome = block_on(metrics().node_metrics(&cluster())).unwrap();
        let samples = outcome.samples().expect("the metrics are there");
        assert_eq!(samples.len(), 100);
        let first = &samples[0];
        // 15 % of 16 cores, give or take the drift.
        let millicores = first.cpu_millicores().expect("a reading") as f64;
        assert!(
            (millicores - 16_000.0 * 0.15).abs() < 16_000.0 * 0.05,
            "{millicores}"
        );
        assert!(first.memory_bytes().unwrap() > NODE_BYTES / 8);
    }

    #[test]
    fn every_pod_gets_a_reading_in_its_namespace() {
        let outcome =
            block_on(metrics().pod_metrics(&cluster(), &NamespaceSelection::All)).unwrap();
        let samples = outcome.samples().expect("the metrics are there");
        assert_eq!(samples.len(), 3_000);
        let MetricsSubject::Pod { pod } = &samples[7].subject else {
            panic!("a pod sample");
        };
        assert_eq!(pod.name.as_ref(), "load-00007");
        assert_eq!(pod.namespace.as_deref(), Some("oxikube-load-7"));
        assert!(samples[7].cpu_millicores().unwrap() >= 20);
    }

    #[test]
    fn a_namespace_selection_narrows_the_pods() {
        let selection = NamespaceSelection::single("oxikube-load-3");
        let outcome = block_on(metrics().pod_metrics(&cluster(), &selection)).unwrap();
        let samples = outcome.samples().expect("the metrics are there");
        assert_eq!(samples.len(), 375);
        assert!(
            samples
                .iter()
                .all(|s| matches!(&s.subject, MetricsSubject::Pod { pod } if pod.namespace.as_deref() == Some("oxikube-load-3")))
        );
    }

    #[test]
    fn a_reading_does_not_jump_between_two_polls() {
        let ts = Timestamp::now();
        let (cpu, _) = pod_reading(42, ts);
        let (later, _) = pod_reading(42, ts + std::time::Duration::from_secs(15));
        let moved = later.abs_diff(cpu) as f64 / cpu as f64;
        assert!(moved < 0.05, "the reading moved by {moved}");
    }
}
