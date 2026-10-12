//! [`NodeTally`]: nodes by readiness and what they can allocate, kept one node at a time.

use std::collections::HashMap;
use std::sync::Arc;

use oxikube_domain::{NodeSummary, Quantity, Resource};

/// How many nodes are ready, not ready and cordoned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NodeCounts {
    /// `Ready` condition `True`.
    pub ready: usize,
    /// `Ready` condition `False`.
    pub not_ready: usize,
    /// No `Ready` condition, or `Unknown` (the kubelet stopped reporting).
    pub unknown: usize,
    /// Cordoned (`spec.unschedulable`), whatever their readiness.
    pub cordoned: usize,
}

impl NodeCounts {
    /// Every node counted.
    pub fn total(&self) -> usize {
        self.ready + self.not_ready + self.unknown
    }

    /// The nodes that are not ready (`False` or unknown).
    pub fn unhealthy(&self) -> usize {
        self.not_ready + self.unknown
    }
}

/// The sum of the nodes' `status.allocatable`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Capacity {
    /// CPU, in millicores.
    pub cpu_millis: i64,
    /// Memory, in bytes.
    pub memory_bytes: i64,
    /// Pods.
    pub pods: i64,
}

impl Capacity {
    fn add(&mut self, other: &Capacity) {
        self.cpu_millis = self.cpu_millis.saturating_add(other.cpu_millis);
        self.memory_bytes = self.memory_bytes.saturating_add(other.memory_bytes);
        self.pods = self.pods.saturating_add(other.pods);
    }

    fn sub(&mut self, other: &Capacity) {
        self.cpu_millis = self.cpu_millis.saturating_sub(other.cpu_millis).max(0);
        self.memory_bytes = self.memory_bytes.saturating_sub(other.memory_bytes).max(0);
        self.pods = self.pods.saturating_sub(other.pods).max(0);
    }
}

/// Whether a node is ready.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Readiness {
    Ready,
    NotReady,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Contribution {
    readiness: Readiness,
    cordoned: bool,
    allocatable: Capacity,
}

impl Contribution {
    fn of(summary: &NodeSummary) -> Self {
        let readiness = if summary.is_ready() {
            Readiness::Ready
        } else if summary.status.starts_with("NotReady") {
            Readiness::NotReady
        } else {
            Readiness::Unknown
        };
        let amount = |q: &Option<Quantity>, milli: bool| {
            q.as_ref().map_or(0, |q| {
                let v = if milli { q.milli_value() } else { q.value() };
                i64::try_from(v).unwrap_or(i64::MAX)
            })
        };
        Self {
            readiness,
            cordoned: !summary.schedulable,
            allocatable: Capacity {
                cpu_millis: amount(&summary.allocatable_cpu, true),
                memory_bytes: amount(&summary.allocatable_memory, false),
                pods: amount(&summary.allocatable_pods, false),
            },
        }
    }
}

/// Nodes by readiness, and the sum of what they can allocate, kept incrementally by node name.
#[derive(Debug, Default, Clone)]
pub struct NodeTally {
    by_name: HashMap<Arc<str>, Contribution>,
    counts: NodeCounts,
    capacity: Capacity,
}

impl NodeTally {
    /// The tally of `nodes` in one pass.
    pub fn of<'a>(nodes: impl IntoIterator<Item = &'a Resource>) -> Self {
        let mut tally = Self::default();
        for node in nodes {
            tally.upsert(node);
        }
        tally
    }

    /// Counts `node`, replacing what it counted before. Objects that are not Nodes are ignored.
    pub fn upsert(&mut self, node: &Resource) {
        let Ok(summary) = NodeSummary::from_resource(node) else {
            return;
        };
        let now = Contribution::of(&summary);
        if let Some(before) = self.by_name.insert(summary.name, now) {
            self.subtract(&before);
        }
        match now.readiness {
            Readiness::Ready => self.counts.ready += 1,
            Readiness::NotReady => self.counts.not_ready += 1,
            Readiness::Unknown => self.counts.unknown += 1,
        }
        self.counts.cordoned += usize::from(now.cordoned);
        self.capacity.add(&now.allocatable);
    }

    /// Stops counting the node `name`.
    pub fn remove(&mut self, name: &str) {
        if let Some(before) = self.by_name.remove(name) {
            self.subtract(&before);
        }
    }

    /// Forgets every node.
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    fn subtract(&mut self, before: &Contribution) {
        let slot = match before.readiness {
            Readiness::Ready => &mut self.counts.ready,
            Readiness::NotReady => &mut self.counts.not_ready,
            Readiness::Unknown => &mut self.counts.unknown,
        };
        *slot = slot.saturating_sub(1);
        if before.cordoned {
            self.counts.cordoned = self.counts.cordoned.saturating_sub(1);
        }
        self.capacity.sub(&before.allocatable);
    }

    /// Nodes by readiness.
    pub fn counts(&self) -> NodeCounts {
        self.counts
    }

    /// The sum of the nodes' allocatable amounts.
    pub fn capacity(&self) -> Capacity {
        self.capacity
    }
}
