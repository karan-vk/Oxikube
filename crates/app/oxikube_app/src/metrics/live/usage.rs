//! [`ClusterUsage`]: one cluster's polled samples, the cache the metrics columns read.
//!
//! Each poll lands as one batch ([`apply`](ClusterUsage::apply)) under one write lock: every
//! node (or pod) of the answer gets its reading pushed onto its ring, and every node (or pod)
//! missing from the answer is dropped, so objects that left the cluster (or the namespace
//! selection) are evicted with the poll that no longer lists them. Readers (the table's cells,
//! per visible row) take a read lock and do one hash lookup by borrowed name: no allocation.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use jiff::Timestamp;
use oxikube_domain::metrics::{ContainerUsage, MetricsSample, MetricsSubject, Reading};
use oxikube_domain::{OxiResult, Resource};
use oxikube_ports::MetricsOutcome;
use parking_lot::RwLock;

use super::ring::{SampleRing, UsagePoint};
use super::state::{MetricsKind, MetricsState};
use crate::columns::{Cell, Metric, MetricsSource, UsageThresholds};
use crate::nodes::{NodeUsage, NodeUsageSource};

/// One pod's readings and its containers' latest.
#[derive(Debug, Default)]
struct PodSeries {
    ring: SampleRing,
    containers: Vec<ContainerUsage>,
}

#[derive(Debug, Default)]
struct Data {
    nodes: HashMap<Arc<str>, SampleRing>,
    /// By namespace, then name.
    pods: HashMap<Arc<str>, HashMap<Arc<str>, PodSeries>>,
    states: [MetricsState; 2],
}

impl Data {
    fn pod(&self, namespace: &str, name: &str) -> Option<&PodSeries> {
        self.pods.get(namespace)?.get(name)
    }
}

/// The samples of one cluster: the latest reading and the last 60 per node and pod, each pod's
/// containers, the [`MetricsState`] of each kind and the thresholds the `%` cells are toned by.
///
/// One per cluster for the life of the [`MetricsService`](super::MetricsService) (a reconnect
/// clears it, it is not replaced), so a column provider can hold it. It is the
/// [`MetricsSource`] of the Pods table and the [`NodeUsageSource`] of the Nodes table.
#[derive(Debug, Default)]
pub struct ClusterUsage {
    data: RwLock<Data>,
    thresholds: RwLock<UsageThresholds>,
    epoch: AtomicU64,
}

impl ClusterUsage {
    /// An empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// How the last poll of `kind` went.
    pub fn state(&self, kind: MetricsKind) -> MetricsState {
        self.data.read().states[kind.index()].clone()
    }

    /// A number that moves with every change of the samples or of a state.
    pub fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::Acquire)
    }

    /// The thresholds the `%` cells are toned by.
    pub fn thresholds(&self) -> UsageThresholds {
        *self.thresholds.read()
    }

    /// The latest reading of the node named `node`.
    pub fn node(&self, node: &str) -> Option<UsagePoint> {
        self.data.read().nodes.get(node)?.latest()
    }

    /// The readings of the node named `node`, oldest first (at most 60).
    pub fn node_history(&self, node: &str) -> Vec<UsagePoint> {
        let data = self.data.read();
        data.nodes
            .get(node)
            .map(|ring| ring.iter().collect())
            .unwrap_or_default()
    }

    /// The latest reading of the pod `namespace/name`.
    pub fn pod(&self, namespace: &str, name: &str) -> Option<UsagePoint> {
        self.data.read().pod(namespace, name)?.ring.latest()
    }

    /// The readings of the pod `namespace/name`, oldest first (at most 60).
    pub fn pod_history(&self, namespace: &str, name: &str) -> Vec<UsagePoint> {
        let data = self.data.read();
        data.pod(namespace, name)
            .map(|series| series.ring.iter().collect())
            .unwrap_or_default()
    }

    /// The latest reading of each running container of the pod `namespace/name`.
    pub fn containers(&self, namespace: &str, name: &str) -> Vec<ContainerUsage> {
        let data = self.data.read();
        data.pod(namespace, name)
            .map(|series| series.containers.clone())
            .unwrap_or_default()
    }

    /// Calls `f` with the latest reading of every node, under one read lock. For whole-cluster
    /// passes off the UI thread (the cluster overview's usage totals, E13-S09).
    pub fn visit_nodes(&self, mut f: impl FnMut(&Arc<str>, &UsagePoint)) {
        let data = self.data.read();
        for (name, ring) in &data.nodes {
            if let Some(point) = ring.latest() {
                f(name, &point);
            }
        }
    }

    /// Calls `f` with the namespace, name and latest reading of every pod, under one read lock;
    /// the pods of one namespace come one after the other. For whole-cluster passes off the UI
    /// thread (the cluster overview's top consumers, E13-S09).
    pub fn visit_pods(&self, mut f: impl FnMut(&Arc<str>, &Arc<str>, &UsagePoint)) {
        let data = self.data.read();
        for (namespace, pods) in &data.pods {
            for (name, series) in pods {
                if let Some(point) = series.ring.latest() {
                    f(namespace, name, &point);
                }
            }
        }
    }

    /// How many nodes and pods have readings.
    pub fn counts(&self) -> (usize, usize) {
        let data = self.data.read();
        (data.nodes.len(), data.pods.values().map(HashMap::len).sum())
    }

    pub(super) fn set_thresholds(&self, thresholds: UsageThresholds) {
        let changed = {
            let mut current = self.thresholds.write();
            let changed = *current != thresholds;
            *current = thresholds;
            changed
        };
        if changed {
            self.bump();
        }
    }

    /// Marks `kind` as asked for (only from `Idle`: a poll in a known state keeps it).
    pub(super) fn set_loading(&self, kind: MetricsKind) {
        let mut data = self.data.write();
        let state = &mut data.states[kind.index()];
        if *state == MetricsState::Idle {
            *state = MetricsState::Loading;
            drop(data);
            self.bump();
        }
    }

    /// Records one poll of `kind`, answered at `at`.
    pub fn apply(&self, kind: MetricsKind, at: Timestamp, result: &OxiResult<MetricsOutcome>) {
        let mut data = self.data.write();
        let state = match result {
            Ok(MetricsOutcome::Available(samples)) => {
                match kind {
                    MetricsKind::Nodes => apply_nodes(&mut data.nodes, samples),
                    MetricsKind::Pods => apply_pods(&mut data.pods, samples),
                }
                MetricsState::Available { at }
            }
            Ok(MetricsOutcome::Unavailable(reason)) => {
                match kind {
                    MetricsKind::Nodes => data.nodes.clear(),
                    MetricsKind::Pods => data.pods.clear(),
                }
                MetricsState::Unavailable(*reason)
            }
            Err(error) => MetricsState::Failed {
                message: error.message().into(),
            },
        };
        data.states[kind.index()] = state;
        drop(data);
        self.bump();
    }

    /// Forgets the pods (the namespace selection changed: the next poll reads the new one).
    pub(super) fn clear_pods(&self) {
        let mut data = self.data.write();
        data.pods.clear();
        drop(data);
        self.bump();
    }

    /// Forgets everything (the cluster disconnected).
    pub(super) fn clear(&self) {
        let mut data = self.data.write();
        data.nodes.clear();
        data.pods.clear();
        data.states = Default::default();
        drop(data);
        self.bump();
    }

    fn bump(&self) {
        self.epoch.fetch_add(1, Ordering::AcqRel);
    }
}

fn point(sample: &MetricsSample) -> UsagePoint {
    UsagePoint::new(sample.ts, sample.cpu, sample.memory)
}

fn apply_nodes(nodes: &mut HashMap<Arc<str>, SampleRing>, samples: &[MetricsSample]) {
    let mut old = std::mem::take(nodes);
    nodes.reserve(samples.len());
    for sample in samples {
        let MetricsSubject::Node { node } = &sample.subject else {
            continue;
        };
        let mut ring = old.remove(node).unwrap_or_default();
        ring.push(point(sample));
        nodes.insert(node.clone(), ring);
    }
}

fn apply_pods(
    pods: &mut HashMap<Arc<str>, HashMap<Arc<str>, PodSeries>>,
    samples: &[MetricsSample],
) {
    let mut old = std::mem::take(pods);
    for sample in samples {
        let MetricsSubject::Pod { pod } = &sample.subject else {
            continue;
        };
        let Some(namespace) = pod.namespace.as_ref() else {
            continue;
        };
        let mut series = old
            .get_mut(namespace)
            .and_then(|in_ns| in_ns.remove(&pod.name))
            .unwrap_or_default();
        series.ring.push(point(sample));
        series.containers.clone_from(&sample.containers);
        pods.entry(namespace.clone())
            .or_default()
            .insert(pod.name.clone(), series);
    }
}

/// `250m`: CPU in whole millicores, as k9s and `kubectl top` print it.
pub fn cpu_text(nanocores: u64) -> String {
    format!("{}m", nanocores / 1_000_000)
}

/// A reading as `text` formats it, or `n/a (<reason>)` for a missing one.
pub(crate) fn reading_text(reading: Reading, text: fn(u64) -> String) -> String {
    match reading {
        Reading::Value(v) => text(v),
        Reading::Missing(why) => format!("n/a ({})", super::state::reason_text(why)),
    }
}

/// `12.5%`, or `-` when there is no figure.
pub(crate) fn percent_text(percent: Option<f64>) -> String {
    percent.map_or_else(|| "-".to_owned(), |p| format!("{p:.1}%"))
}

/// `128Mi`: memory in whole mebibytes (`512Ki` below one), as k9s and `kubectl top` print it.
pub fn memory_text(bytes: u64) -> String {
    const KI: u64 = 1024;
    const MI: u64 = 1024 * 1024;
    if bytes >= MI {
        format!("{}Mi", bytes / MI)
    } else {
        format!("{}Ki", bytes / KI)
    }
}

/// The usage cell of `metric` from `point`: the text, and the exact quantity as the sort key.
pub fn usage_cell(point: &UsagePoint, metric: Metric) -> Option<Cell<'static>> {
    match metric {
        Metric::Cpu => Some(Cell::quantity(
            cpu_text(point.cpu_nanocores?),
            point.cpu_quantity()?,
        )),
        Metric::Memory => Some(Cell::quantity(
            memory_text(point.memory_bytes?),
            point.memory_quantity()?,
        )),
    }
}

impl MetricsSource for ClusterUsage {
    fn cell(&self, object: &Resource, metric: Metric, _: Timestamp) -> Option<Cell<'static>> {
        let namespace = object.meta.namespace.as_deref()?;
        usage_cell(&self.pod(namespace, &object.meta.name)?, metric)
    }

    fn thresholds(&self) -> UsageThresholds {
        ClusterUsage::thresholds(self)
    }

    fn epoch(&self) -> u64 {
        ClusterUsage::epoch(self)
    }
}

impl NodeUsageSource for ClusterUsage {
    fn usage(&self, node: &str) -> Option<NodeUsage> {
        let point = self.node(node)?;
        Some(NodeUsage {
            cpu_millis: i64::try_from(point.cpu_nanocores? / 1_000_000).ok()?,
            memory_bytes: i64::try_from(point.memory_bytes?).ok()?,
        })
    }

    fn epoch(&self) -> u64 {
        ClusterUsage::epoch(self)
    }

    fn thresholds(&self) -> UsageThresholds {
        ClusterUsage::thresholds(self)
    }
}
