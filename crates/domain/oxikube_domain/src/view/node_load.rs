//! [`Demand`] and [`PodPlacement`]: what the pods on a node ask of it, for the "Allocated
//! resources" block of the node detail and the Pods column of the node table.
//!
//! The rules are `kubectl describe node`'s: only pods that are not `Succeeded` or `Failed` count,
//! a pod's request is the sum over its containers raised to the largest init container (restartable
//! init containers, the sidecars, add to the sum and to every init container after them) plus the
//! pod's `overhead`. A [`Demand`] adds and subtracts exactly, so a store can keep one per node and
//! apply a pod's change in O(containers) instead of summing the pods again.

use std::ops::{Add, AddAssign, Sub, SubAssign};
use std::sync::Arc;

use super::pod::{PodPhase, is_restartable};
use super::{arc_of, arr_of, str_of, sub};
use crate::json::JsonRef;
use crate::quantity::Quantity;
use crate::resource::Resource;

/// The sum of what some pods request and are limited to, and how many pods that is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Demand {
    /// Pods counted.
    pub pods: u32,
    /// CPU requests, in millicores.
    pub cpu_requests: i64,
    /// CPU limits, in millicores.
    pub cpu_limits: i64,
    /// Memory requests, in bytes.
    pub memory_requests: i64,
    /// Memory limits, in bytes.
    pub memory_limits: i64,
}

impl Demand {
    /// Nothing.
    pub const ZERO: Demand = Demand {
        pods: 0,
        cpu_requests: 0,
        cpu_limits: 0,
        memory_requests: 0,
        memory_limits: 0,
    };

    /// Whether no pod is counted.
    pub fn is_empty(&self) -> bool {
        self.pods == 0
    }
}

impl Add for Demand {
    type Output = Demand;

    fn add(self, other: Demand) -> Demand {
        Demand {
            pods: self.pods.saturating_add(other.pods),
            cpu_requests: self.cpu_requests.saturating_add(other.cpu_requests),
            cpu_limits: self.cpu_limits.saturating_add(other.cpu_limits),
            memory_requests: self.memory_requests.saturating_add(other.memory_requests),
            memory_limits: self.memory_limits.saturating_add(other.memory_limits),
        }
    }
}

impl Sub for Demand {
    type Output = Demand;

    fn sub(self, other: Demand) -> Demand {
        Demand {
            pods: self.pods.saturating_sub(other.pods),
            cpu_requests: self.cpu_requests.saturating_sub(other.cpu_requests).max(0),
            cpu_limits: self.cpu_limits.saturating_sub(other.cpu_limits).max(0),
            memory_requests: self
                .memory_requests
                .saturating_sub(other.memory_requests)
                .max(0),
            memory_limits: self
                .memory_limits
                .saturating_sub(other.memory_limits)
                .max(0),
        }
    }
}

impl AddAssign for Demand {
    fn add_assign(&mut self, other: Demand) {
        *self = *self + other;
    }
}

impl SubAssign for Demand {
    fn sub_assign(&mut self, other: Demand) {
        *self = *self - other;
    }
}

/// A pod that occupies a node: the node's name and what the pod asks of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PodPlacement {
    /// `spec.nodeName`.
    pub node: Arc<str>,
    /// The pod's own demand (`pods` is `1`).
    pub demand: Demand,
}

impl PodPlacement {
    /// Where `pod` runs and what it takes there, or `None` for a pod that is not a Pod, has no
    /// node yet, or has finished (`Succeeded` or `Failed`).
    pub fn of_pod(pod: &Resource) -> Option<PodPlacement> {
        if !pod.kind.is_pod() || pod.is_partial() {
            return None;
        }
        let spec = sub(pod.json(), "spec");
        let node = arc_of(spec, "nodeName")?;
        let phase = PodPhase::parse(str_of(sub(pod.json(), "status"), "phase"));
        if matches!(phase, PodPhase::Succeeded | PodPhase::Failed) {
            return None;
        }
        let cpu = effective(spec, "cpu", Unit::Milli);
        let memory = effective(spec, "memory", Unit::Value);
        Some(PodPlacement {
            node,
            demand: Demand {
                pods: 1,
                cpu_requests: cpu.requests,
                cpu_limits: cpu.limits,
                memory_requests: memory.requests,
                memory_limits: memory.limits,
            },
        })
    }
}

#[derive(Clone, Copy)]
enum Unit {
    Milli,
    Value,
}

/// A pair of totals for one resource.
#[derive(Clone, Copy, Default)]
struct Pair {
    requests: i64,
    limits: i64,
}

impl Pair {
    fn plus(self, other: Pair) -> Pair {
        Pair {
            requests: self.requests.saturating_add(other.requests),
            limits: self.limits.saturating_add(other.limits),
        }
    }

    fn max(self, other: Pair) -> Pair {
        Pair {
            requests: self.requests.max(other.requests),
            limits: self.limits.max(other.limits),
        }
    }
}

/// What one container asks for `resource`, in `unit`.
fn container(c: JsonRef<'_>, resource: &str, unit: Unit) -> Pair {
    let resources = sub(c, "resources");
    let read = |section: &str| {
        str_of(sub(resources, section), resource)
            .and_then(|q| Quantity::parse(q).ok())
            .map_or(0, |q| to_i64(&q, unit))
    };
    let limits = read("limits");
    // The API server defaults a missing request to the limit; do the same for an object that was
    // never admitted (a dry-run or a fixture).
    let requests = match str_of(sub(resources, "requests"), resource) {
        Some(_) => read("requests"),
        None => limits,
    };
    Pair { requests, limits }
}

fn to_i64(q: &Quantity, unit: Unit) -> i64 {
    let v = match unit {
        Unit::Milli => q.milli_value(),
        Unit::Value => q.value(),
    };
    i64::try_from(v.max(0)).unwrap_or(i64::MAX)
}

/// The pod-level total of `resource`, by the scheduler's rule (see the module docs).
fn effective(spec: JsonRef<'_>, resource: &str, unit: Unit) -> Pair {
    let mut total = arr_of(spec, "containers")
        .iter()
        .fold(Pair::default(), |sum, c| {
            sum.plus(container(c, resource, unit))
        });
    let mut sidecars = Pair::default();
    let mut init_peak = Pair::default();
    for init in arr_of(spec, "initContainers").iter() {
        let pair = container(init, resource, unit);
        if is_restartable(init) {
            total = total.plus(pair);
            sidecars = sidecars.plus(pair);
            init_peak = init_peak.max(sidecars);
        } else {
            init_peak = init_peak.max(pair.plus(sidecars));
        }
    }
    let overhead = str_of(sub(spec, "overhead"), resource)
        .and_then(|q| Quantity::parse(q).ok())
        .map_or(0, |q| to_i64(&q, unit));
    let peak = total.max(init_peak);
    Pair {
        requests: peak.requests.saturating_add(overhead),
        limits: peak.limits.saturating_add(overhead),
    }
}
