//! [`NodeAllocation`]: the "Allocated resources" block of `kubectl describe node` as numbers a
//! view can draw bars from: requests and limits of the pods on a node against what the node
//! can allocate, for CPU, memory and the pod count.

use super::node::NodeSummary;
use super::node_load::Demand;
use crate::quantity::Quantity;

/// What an [`AllocationLine`] measures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AllocationResource {
    /// CPU, in millicores.
    Cpu,
    /// Memory, in bytes.
    Memory,
    /// The pod count.
    Pods,
}

impl AllocationResource {
    /// The label of the line: `CPU`, `Memory` or `Pods`.
    pub const fn title(self) -> &'static str {
        match self {
            Self::Cpu => "CPU",
            Self::Memory => "Memory",
            Self::Pods => "Pods",
        }
    }

    /// `n` in this resource's unit as text: `250m`, `1.5 GiB`, `12`.
    pub fn format(self, n: i64) -> String {
        match self {
            Self::Cpu => Quantity::from_milli(n).human_cpu(),
            Self::Memory => Quantity::from_value(n).human_bytes(),
            Self::Pods => n.to_string(),
        }
    }
}

/// One resource of a node: what the pods on it request and are limited to, against what it can
/// allocate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AllocationLine {
    /// The resource.
    pub resource: AllocationResource,
    /// `status.allocatable` of the resource; `None` when the node reports none.
    pub allocatable: Option<i64>,
    /// The sum of the pods' requests (for pods: how many there are).
    pub requests: i64,
    /// The sum of the pods' limits; `None` for the pod count, which has none.
    pub limits: Option<i64>,
}

impl AllocationLine {
    /// Requests as a percentage of the allocatable amount, rounded down as kubectl does; `None`
    /// when the node reports no allocatable amount.
    pub fn request_percent(&self) -> Option<u32> {
        percent(self.requests, self.allocatable)
    }

    /// Limits as a percentage of the allocatable amount (can exceed 100: limits overcommit).
    pub fn limit_percent(&self) -> Option<u32> {
        percent(self.limits?, self.allocatable)
    }

    /// How full the bar is for requests, in `0.0..=1.0`.
    pub fn request_fill(&self) -> f32 {
        fill(self.requests, self.allocatable)
    }

    /// How far the limits mark sits, in `0.0..=1.0` (clamped; the text says the real figure).
    pub fn limit_fill(&self) -> f32 {
        fill(self.limits.unwrap_or(0), self.allocatable)
    }

    /// Whether the requests need more than the node can allocate.
    pub fn is_overcommitted(&self) -> bool {
        self.allocatable.is_some_and(|a| self.requests > a)
    }

    /// `750m (18%)`: requests with their percentage.
    pub fn requests_text(&self) -> String {
        with_percent(self.resource, self.requests, self.request_percent())
    }

    /// `1500m (37%)`: limits with their percentage; empty for the pod count.
    pub fn limits_text(&self) -> String {
        match self.limits {
            Some(limits) => with_percent(self.resource, limits, self.limit_percent()),
            None => String::new(),
        }
    }

    /// The allocatable amount as text, `-` when the node reports none.
    pub fn allocatable_text(&self) -> String {
        self.allocatable
            .map_or_else(|| "-".to_owned(), |a| self.resource.format(a))
    }
}

fn percent(amount: i64, total: Option<i64>) -> Option<u32> {
    let total = total.filter(|t| *t > 0)?;
    let pct = i128::from(amount.max(0)) * 100 / i128::from(total);
    Some(u32::try_from(pct).unwrap_or(u32::MAX))
}

fn fill(amount: i64, total: Option<i64>) -> f32 {
    match total.filter(|t| *t > 0) {
        Some(total) => (amount.max(0) as f64 / total as f64).clamp(0.0, 1.0) as f32,
        None => 0.0,
    }
}

fn with_percent(resource: AllocationResource, amount: i64, percent: Option<u32>) -> String {
    match percent {
        Some(p) => format!("{} ({p}%)", resource.format(amount)),
        None => resource.format(amount),
    }
}

/// CPU, memory and pods of one node: [`NodeAllocation::new`] joins its allocatable amounts with
/// the [`Demand`] of the pods on it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NodeAllocation {
    /// CPU.
    pub cpu: AllocationLine,
    /// Memory.
    pub memory: AllocationLine,
    /// The pod count against `status.allocatable.pods`.
    pub pods: AllocationLine,
}

impl NodeAllocation {
    /// The allocation of `node` under `demand`.
    pub fn new(node: &NodeSummary, demand: &Demand) -> Self {
        let amount = |q: &Option<Quantity>, milli: bool| {
            q.as_ref().map(|q| {
                let v = if milli { q.milli_value() } else { q.value() };
                i64::try_from(v).unwrap_or(i64::MAX)
            })
        };
        Self {
            cpu: AllocationLine {
                resource: AllocationResource::Cpu,
                allocatable: amount(&node.allocatable_cpu, true),
                requests: demand.cpu_requests,
                limits: Some(demand.cpu_limits),
            },
            memory: AllocationLine {
                resource: AllocationResource::Memory,
                allocatable: amount(&node.allocatable_memory, false),
                requests: demand.memory_requests,
                limits: Some(demand.memory_limits),
            },
            pods: AllocationLine {
                resource: AllocationResource::Pods,
                allocatable: amount(&node.allocatable_pods, false),
                requests: i64::from(demand.pods),
                limits: None,
            },
        }
    }

    /// The three lines in display order.
    pub fn lines(&self) -> [&AllocationLine; 3] {
        [&self.cpu, &self.memory, &self.pods]
    }
}
