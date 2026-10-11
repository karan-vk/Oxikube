//! [`RawNode`] / [`RawPod`] to [`MetricsSample`], with the maths on the domain [`Quantity`].
//!
//! kdash parses usage with string helpers that mishandle many inputs (`129e6`, `1.5Gi`, `M`
//! versus `Mi`, leading signs); here every quantity goes through [`Quantity::parse`], and a
//! pod's containers are summed as exact quantities before one conversion to the port's units
//! (nanocores, bytes). An unreadable quantity becomes a visible [`Reading::Missing`] on that
//! one reading, never a dropped sample (ADR 0011).

use jiff::Timestamp;
use oxikube_domain::Quantity;
use oxikube_domain::ids::{ClusterId, Gvk, ResourceRef};
use oxikube_domain::metrics::{
    ContainerUsage, MetricsSample, MetricsSubject, MissingReason, Reading,
};

use super::raw::{RawNode, RawPod, RawUsage};

/// Which usage field a reading comes from, with its unit conversion.
#[derive(Clone, Copy)]
enum Dimension {
    /// Nanocores.
    Cpu,
    /// Bytes.
    Memory,
}

impl Dimension {
    fn text(self, usage: &RawUsage) -> Option<&str> {
        match self {
            Dimension::Cpu => usage.cpu.as_deref(),
            Dimension::Memory => usage.memory.as_deref(),
        }
    }

    /// The quantity in the port's unit, if it is a non-negative number that fits `u64`.
    fn to_units(self, quantity: &Quantity) -> Option<u64> {
        let units = match self {
            Dimension::Cpu => quantity.checked_nanos()?,
            Dimension::Memory => quantity.value(),
        };
        u64::try_from(units).ok()
    }
}

/// One usage field parsed once: the exact quantity, or `None` when absent or unreadable.
fn parse(usage: &RawUsage, dimension: Dimension) -> Option<Quantity> {
    Quantity::parse(dimension.text(usage)?).ok()
}

/// A single reading in the port's unit (unreadable, negative or oversized values are `Other`).
fn reading_of(quantity: Option<&Quantity>, dimension: Dimension) -> Reading {
    match quantity.and_then(|q| dimension.to_units(q)) {
        Some(units) => Reading::Value(units),
        None => Reading::Missing(MissingReason::Other),
    }
}

/// A pod's total for one dimension, from the quantities already parsed per container: the exact
/// sum, or missing when any container is unreadable (a partial sum would understate the pod).
fn total_of(parsed: &[Option<Quantity>], dimension: Dimension) -> Reading {
    if parsed.is_empty() {
        // metrics-server has registered the pod but no container has been scraped yet.
        return Reading::Missing(MissingReason::NotYetAvailable);
    }
    let sum = parsed
        .iter()
        .try_fold(Quantity::ZERO, |total, q| total.checked_add(q.as_ref()?));
    reading_of(sum.as_ref(), dimension)
}

/// The sample of one node. `fetched_at` stands in when the server sent no readable timestamp.
pub(super) fn node_sample(raw: RawNode, fetched_at: Timestamp) -> MetricsSample {
    let cpu = parse(&raw.usage, Dimension::Cpu);
    let memory = parse(&raw.usage, Dimension::Memory);
    MetricsSample {
        subject: MetricsSubject::Node {
            node: raw.name.into(),
        },
        ts: raw.timestamp.unwrap_or(fetched_at),
        window: raw.window,
        cpu: reading_of(cpu.as_ref(), Dimension::Cpu),
        memory: reading_of(memory.as_ref(), Dimension::Memory),
        containers: Vec::new(),
    }
}

/// The sample of one pod, summed over its containers.
pub(super) fn pod_sample(cluster: &ClusterId, raw: RawPod, fetched_at: Timestamp) -> MetricsSample {
    let pod = ResourceRef::namespaced(
        cluster.clone(),
        Gvk::new("", "v1", "Pod"),
        raw.namespace,
        raw.name,
    );
    // Each quantity is parsed once; the pod total and the container entries both use it.
    let cpus: Vec<Option<Quantity>> = raw
        .containers
        .iter()
        .map(|c| parse(&c.usage, Dimension::Cpu))
        .collect();
    let memories: Vec<Option<Quantity>> = raw
        .containers
        .iter()
        .map(|c| parse(&c.usage, Dimension::Memory))
        .collect();
    let (cpu, memory) = (
        total_of(&cpus, Dimension::Cpu),
        total_of(&memories, Dimension::Memory),
    );
    // One entry per container the server lists (init containers are never served), kept so the
    // detail panel can break the pod's total down. A container may read while the total does
    // not: the total is all-or-nothing, the entries are independent.
    let containers = raw
        .containers
        .into_iter()
        .zip(cpus.iter().zip(&memories))
        .map(|(c, (cpu, memory))| ContainerUsage {
            name: c.name.into(),
            cpu: reading_of(cpu.as_ref(), Dimension::Cpu),
            memory: reading_of(memory.as_ref(), Dimension::Memory),
        })
        .collect();
    MetricsSample {
        subject: MetricsSubject::Pod { pod },
        ts: raw.timestamp.unwrap_or(fetched_at),
        window: raw.window,
        cpu,
        memory,
        containers,
    }
}
