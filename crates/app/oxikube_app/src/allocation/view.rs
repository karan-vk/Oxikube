//! [`AllocationView`]: the rows of a node's or a namespace's allocation section.

use std::sync::Arc;

use oxikube_domain::Quantity;
use oxikube_domain::utilisation::{QuotaCaps, ResourceAmounts, UnsetPods};

use super::usage::UsageState;
use super::{AllocationKind, AllocationRow, UnsetCount, Usage};
use crate::{NamespaceAllocation, NodeAllocation};

/// What the capacity of the rows is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapacityKind {
    /// A node's `status.allocatable`.
    Allocatable,
    /// The tightest `ResourceQuota` of a namespace.
    Quota,
}

/// The allocation section of a node or a namespace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllocationView {
    /// What the rows are measured against.
    pub capacity: CapacityKind,
    /// The rows, in display order.
    pub rows: Vec<AllocationRow>,
    /// Pods the sums cover (those that hold resources).
    pub pods: u32,
    /// Notes under the rows: how much of the usage is covered, quotas left out.
    pub notes: Vec<String>,
}

impl AllocationView {
    /// The allocation of a node: per resource, the pods' requests and limits and the node's
    /// usage against `status.allocatable`.
    ///
    /// Rows: CPU, memory and pods always; ephemeral storage when the node reports it or a pod
    /// asks for it; one row per configured GPU resource the node offers or a pod asks for.
    pub fn for_node(allocation: &NodeAllocation, usage: &UsageState) -> Self {
        let NodeAllocation {
            node,
            requests,
            limits,
            pods,
            unset,
        } = allocation;
        let allocatable = &node.allocatable;
        let pod_count = Quantity::from_value(i64::from(*pods));
        let mut rows = vec![
            AllocationRow {
                capacity: some_if(!allocatable.cpu.is_zero(), allocatable.cpu),
                limits_capacity: some_if(!allocatable.cpu.is_zero(), allocatable.cpu),
                ..row(
                    AllocationKind::Cpu,
                    requests.cpu,
                    limits.cpu,
                    usage.cpu(),
                    cpu_unset(unset),
                )
            },
            AllocationRow {
                capacity: some_if(!allocatable.memory.is_zero(), allocatable.memory),
                limits_capacity: some_if(!allocatable.memory.is_zero(), allocatable.memory),
                ..row(
                    AllocationKind::Memory,
                    requests.memory,
                    limits.memory,
                    usage.memory(),
                    memory_unset(unset),
                )
            },
            AllocationRow {
                kind: AllocationKind::Pods,
                capacity: node.max_pods.map(|n| Quantity::from_value(to_i64(n))),
                limits_capacity: None,
                requests: pod_count,
                limits: None,
                usage: Usage::NotApplicable,
                unset: UnsetCount::default(),
            },
        ];
        if !allocatable.ephemeral.is_zero()
            || !requests.ephemeral.is_zero()
            || !limits.ephemeral.is_zero()
        {
            rows.push(AllocationRow {
                capacity: some_if(!allocatable.ephemeral.is_zero(), allocatable.ephemeral),
                limits_capacity: some_if(!allocatable.ephemeral.is_zero(), allocatable.ephemeral),
                ..row(
                    AllocationKind::EphemeralStorage,
                    requests.ephemeral,
                    limits.ephemeral,
                    Usage::NotReported,
                    ephemeral_unset(unset),
                )
            });
        }
        for name in gpu_names(allocatable, requests, limits) {
            let offered = allocatable.gpu_count(&name);
            let cap = some_if(offered > 0, Quantity::from_value(to_i64(offered)));
            rows.push(AllocationRow {
                capacity: cap,
                limits_capacity: cap,
                ..gpu_row(&name, requests, limits)
            });
        }
        Self {
            capacity: CapacityKind::Allocatable,
            rows,
            pods: *pods,
            notes: Vec::new(),
        }
    }

    /// The allocation of a namespace: the totals over its pods, measured against the tightest
    /// unscoped `ResourceQuota` caps where there are any (`scoped_quotas` counts the quotas
    /// left out because they cap only some pods).
    ///
    /// Rows: CPU and memory always; ephemeral storage, pods and GPUs when a quota caps them or a
    /// pod uses them. Without quotas the rows carry totals and no capacity (no bar).
    pub fn for_namespace(
        allocation: &NamespaceAllocation,
        caps: &QuotaCaps,
        scoped_quotas: usize,
        usage: &UsageState,
    ) -> Self {
        let NamespaceAllocation {
            requests,
            limits,
            pods,
            unset,
            ..
        } = allocation;
        let mut rows = vec![
            AllocationRow {
                capacity: caps.requests.cpu,
                limits_capacity: caps.limits.cpu,
                ..row(
                    AllocationKind::Cpu,
                    requests.cpu,
                    limits.cpu,
                    namespace_usage_of(usage, *pods, true),
                    cpu_unset(unset),
                )
            },
            AllocationRow {
                capacity: caps.requests.memory,
                limits_capacity: caps.limits.memory,
                ..row(
                    AllocationKind::Memory,
                    requests.memory,
                    limits.memory,
                    namespace_usage_of(usage, *pods, false),
                    memory_unset(unset),
                )
            },
        ];
        let ephemeral_used = !requests.ephemeral.is_zero() || !limits.ephemeral.is_zero();
        if ephemeral_used || caps.requests.ephemeral.is_some() || caps.limits.ephemeral.is_some() {
            rows.push(AllocationRow {
                capacity: caps.requests.ephemeral,
                limits_capacity: caps.limits.ephemeral,
                ..row(
                    AllocationKind::EphemeralStorage,
                    requests.ephemeral,
                    limits.ephemeral,
                    Usage::NotReported,
                    ephemeral_unset(unset),
                )
            });
        }
        if caps.pods.is_some() || *pods > 0 {
            rows.push(AllocationRow {
                kind: AllocationKind::Pods,
                capacity: caps.pods.map(|n| Quantity::from_value(to_i64(n))),
                limits_capacity: None,
                requests: Quantity::from_value(i64::from(*pods)),
                limits: None,
                usage: Usage::NotApplicable,
                unset: UnsetCount::default(),
            });
        }
        let mut names: Vec<Arc<str>> = requests
            .gpu
            .keys()
            .chain(limits.gpu.keys())
            .cloned()
            .collect();
        names.extend(caps.requests.gpu.keys().cloned());
        names.sort();
        names.dedup();
        for name in names {
            let cap = caps
                .requests
                .gpu
                .get(&name)
                .map(|n| Quantity::from_value(to_i64(*n)));
            rows.push(AllocationRow {
                capacity: cap,
                limits_capacity: None,
                ..gpu_row(&name, requests, limits)
            });
        }
        let mut notes = Vec::new();
        if let UsageState::Ready(measured) = usage
            && measured.pods_unreadable > 0
        {
            notes.push(format!(
                "usage leaves out {} {} with no reading yet",
                measured.pods_unreadable,
                if measured.pods_unreadable == 1 {
                    "pod"
                } else {
                    "pods"
                }
            ));
        }
        if scoped_quotas > 0 {
            notes.push(format!(
                "{scoped_quotas} scoped {} cap{} only some pods and {} not drawn as {}",
                if scoped_quotas == 1 {
                    "quota"
                } else {
                    "quotas"
                },
                if scoped_quotas == 1 { "s" } else { "" },
                if scoped_quotas == 1 { "is" } else { "are" },
                "a bar",
            ));
        }
        Self {
            capacity: CapacityKind::Quota,
            rows,
            pods: *pods,
            notes,
        }
    }

    /// The row of `kind`, if the view has one.
    pub fn row(&self, kind: &AllocationKind) -> Option<&AllocationRow> {
        self.rows.iter().find(|r| &r.kind == kind)
    }

    /// The word the capacity is called in a heading: `allocatable` or `quota`.
    pub fn capacity_name(&self) -> &'static str {
        match self.capacity {
            CapacityKind::Allocatable => "allocatable",
            CapacityKind::Quota => "quota",
        }
    }
}

/// The usage a namespace's CPU (`cpu`) or memory row shows. Pods with no sample at all
/// (everything in the namespace pending) are not a zero reading.
fn namespace_usage_of(usage: &UsageState, pods: u32, cpu: bool) -> Usage {
    if let UsageState::Ready(measured) = usage
        && measured.pods_reporting == 0
        && measured.pods_unreadable == 0
        && pods > 0
    {
        return Usage::Unavailable(
            "metrics unavailable: no readings for the pods of this namespace yet".into(),
        );
    }
    if cpu { usage.cpu() } else { usage.memory() }
}

fn row(
    kind: AllocationKind,
    requests: Quantity,
    limits: Quantity,
    usage: Usage,
    unset: UnsetCount,
) -> AllocationRow {
    AllocationRow {
        kind,
        capacity: None,
        limits_capacity: None,
        requests,
        limits: Some(limits),
        usage,
        unset,
    }
}

fn gpu_row(name: &Arc<str>, requests: &ResourceAmounts, limits: &ResourceAmounts) -> AllocationRow {
    row(
        AllocationKind::Gpu(name.clone()),
        Quantity::from_value(to_i64(requests.gpu_count(name))),
        Quantity::from_value(to_i64(limits.gpu_count(name))),
        Usage::NotReported,
        UnsetCount::default(),
    )
}

/// GPU resource names the node offers or a pod asks for, sorted.
fn gpu_names(
    allocatable: &ResourceAmounts,
    requests: &ResourceAmounts,
    limits: &ResourceAmounts,
) -> Vec<Arc<str>> {
    let mut names: Vec<Arc<str>> = allocatable
        .gpu
        .keys()
        .chain(requests.gpu.keys())
        .chain(limits.gpu.keys())
        .cloned()
        .collect();
    names.sort();
    names.dedup();
    names
}

fn cpu_unset(unset: &UnsetPods) -> UnsetCount {
    UnsetCount {
        requests: unset.cpu_requests,
        limits: unset.cpu_limits,
    }
}

fn memory_unset(unset: &UnsetPods) -> UnsetCount {
    UnsetCount {
        requests: unset.memory_requests,
        limits: unset.memory_limits,
    }
}

fn ephemeral_unset(unset: &UnsetPods) -> UnsetCount {
    UnsetCount {
        requests: unset.ephemeral_requests,
        limits: unset.ephemeral_limits,
    }
}

fn some_if<T>(condition: bool, value: T) -> Option<T> {
    condition.then_some(value)
}

fn to_i64(n: u64) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}
