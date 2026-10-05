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
use oxikube_domain::metrics::{MetricsSample, MetricsSubject, MissingReason, Reading};

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

/// Sums one dimension over `usages` exactly, or `None` when any entry is absent or unreadable
/// (a partial sum would understate the pod).
fn sum(usages: &[RawUsage], dimension: Dimension) -> Option<Quantity> {
    usages.iter().try_fold(Quantity::ZERO, |total, usage| {
        let quantity = Quantity::parse(dimension.text(usage)?).ok()?;
        total.checked_add(&quantity)
    })
}

fn reading(usages: &[RawUsage], dimension: Dimension) -> Reading {
    if usages.is_empty() {
        // metrics-server has registered the pod but no container has been scraped yet.
        return Reading::Missing(MissingReason::NotYetAvailable);
    }
    match sum(usages, dimension).and_then(|total| dimension.to_units(&total)) {
        Some(units) => Reading::Value(units),
        None => Reading::Missing(MissingReason::Unavailable),
    }
}

/// The sample of one node. `fetched_at` stands in when the server sent no readable timestamp.
pub(super) fn node_sample(raw: RawNode, fetched_at: Timestamp) -> MetricsSample {
    let usage = std::slice::from_ref(&raw.usage);
    MetricsSample {
        subject: MetricsSubject::Node {
            node: raw.name.into(),
        },
        ts: raw.timestamp.unwrap_or(fetched_at),
        window: raw.window,
        cpu: reading(usage, Dimension::Cpu),
        memory: reading(usage, Dimension::Memory),
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
    MetricsSample {
        subject: MetricsSubject::Pod { pod },
        ts: raw.timestamp.unwrap_or(fetched_at),
        window: raw.window,
        cpu: reading(&raw.containers, Dimension::Cpu),
        memory: reading(&raw.containers, Dimension::Memory),
    }
}
