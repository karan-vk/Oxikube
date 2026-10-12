//! Percent maths: usage over a base, `None` when there is no base.

use std::collections::BTreeMap;
use std::sync::Arc;

use super::{PodResources, ResourceAmounts};
use crate::metrics::MetricsSample;
use crate::quantity::Quantity;

/// `usage` as a percent of `base` (`250m` of `1` is `25.0`), or `None` when the figure would
/// mean nothing: `base` is zero (no request or limit set, shown as a dash) or either value is
/// negative.
///
/// Exact where it can be: both values are scaled to nano-units and divided in integers, so
/// the result does not depend on how the quantities are spelled (`500m / 1` equals `500 / 1000`)
/// and grows with `usage` without ever stepping back. Values too large for that fall back to
/// [`Quantity::percent_of`]. The result is a float for display only; compare the quantities,
/// not this.
pub fn percent(usage: &Quantity, base: &Quantity) -> Option<f64> {
    if usage.is_negative() || base.is_negative() || base.is_zero() {
        return None;
    }
    if let (Some(used), Some(total)) = (usage.checked_nanos(), base.checked_nanos())
        && let Some(scaled) = used.checked_mul(100)
    {
        let (whole, rest) = (scaled / total, scaled % total);
        return Some(whole as f64 + rest as f64 / total as f64);
    }
    usage.percent_of(base)
}

/// The four ratios of a pod table: usage over requests and over limits, for CPU and memory
/// (`%CPU/R`, `%CPU/L`, `%MEM/R`, `%MEM/L`). `None` is a dash: no usage reading, or no request
/// or limit set.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Utilisation {
    /// CPU usage as a percent of the CPU request (`%CPU/R`).
    pub cpu_of_requests: Option<f64>,
    /// CPU usage as a percent of the CPU limit (`%CPU/L`).
    pub cpu_of_limits: Option<f64>,
    /// Memory usage as a percent of the memory request (`%MEM/R`).
    pub memory_of_requests: Option<f64>,
    /// Memory usage as a percent of the memory limit (`%MEM/L`).
    pub memory_of_limits: Option<f64>,
}

impl Utilisation {
    /// Usage against what the pod asks for. A missing reading gives dashes for its resource.
    pub fn of(
        cpu_usage: Option<&Quantity>,
        memory_usage: Option<&Quantity>,
        resources: &PodResources,
    ) -> Self {
        let ratio =
            |usage: Option<&Quantity>, base: &Quantity| usage.and_then(|u| percent(u, base));
        Self {
            cpu_of_requests: ratio(cpu_usage, &resources.requests.cpu),
            cpu_of_limits: ratio(cpu_usage, &resources.limits.cpu),
            memory_of_requests: ratio(memory_usage, &resources.requests.memory),
            memory_of_limits: ratio(memory_usage, &resources.limits.memory),
        }
    }

    /// [`Utilisation::of`] for a metrics sample; a [`Reading::Missing`](crate::metrics::Reading)
    /// is a dash (the sample still says why).
    pub fn of_sample(sample: &MetricsSample, resources: &PodResources) -> Self {
        Self::of(
            sample.cpu_quantity().as_ref(),
            sample.memory_quantity().as_ref(),
            resources,
        )
    }
}

/// [`ResourceAmounts`] as a percent of another: a node's requests over its allocatable, a
/// namespace's requests over its quota. Each field is `None` when the base has none of it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AmountPercents {
    /// CPU percent.
    pub cpu: Option<f64>,
    /// Memory percent.
    pub memory: Option<f64>,
    /// Ephemeral storage percent.
    pub ephemeral: Option<f64>,
    /// Per GPU resource name present in either operand.
    pub gpu: BTreeMap<Arc<str>, Option<f64>>,
}

impl ResourceAmounts {
    /// `self` as a percent of `base`, field by field (see [`percent`]).
    pub fn percent_of(&self, base: &ResourceAmounts) -> AmountPercents {
        let mut gpu = BTreeMap::new();
        for name in self.gpu.keys().chain(base.gpu.keys()) {
            let used = self.gpu_count(name);
            let total = base.gpu_count(name);
            let value = (total > 0).then(|| used as f64 * 100.0 / total as f64);
            gpu.insert(name.clone(), value);
        }
        AmountPercents {
            cpu: percent(&self.cpu, &base.cpu),
            memory: percent(&self.memory, &base.memory),
            ephemeral: percent(&self.ephemeral, &base.ephemeral),
            gpu,
        }
    }
}
