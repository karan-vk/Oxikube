//! [`AllocationView`]: the rows of a node's or a namespace's allocation section.

use std::sync::Arc;

use oxikube_domain::Quantity;
use oxikube_domain::utilisation::{QuotaCaps, ResourceAmounts, UnsetPods};

use super::usage::UsageState;
use super::{AllocationKind, AllocationRow, CapacityKind, UnsetCount, Usage};
use crate::{NamespaceAllocation, NodeAllocation};

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

/// The two caps of a row: on requests, and on limits.
#[derive(Clone, Copy, Default)]
struct Caps {
    requests: Option<Quantity>,
    limits: Option<Quantity>,
}

impl Caps {
    /// One capacity for both: a node's allocatable. Zero is "the node does not report it".
    fn node(amount: Quantity) -> Self {
        let cap = (!amount.is_zero()).then_some(amount);
        Self {
            requests: cap,
            limits: cap,
        }
    }
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
        let have = &node.allocatable;
        let make = Maker(CapacityKind::Allocatable);
        let mut rows = vec![
            make.row(
                AllocationKind::Cpu,
                (requests.cpu, limits.cpu),
                Caps::node(have.cpu),
                usage.cpu(),
                cpu_unset(unset),
            ),
            make.row(
                AllocationKind::Memory,
                (requests.memory, limits.memory),
                Caps::node(have.memory),
                usage.memory(),
                memory_unset(unset),
            ),
            make.pods(*pods, node.max_pods),
        ];
        if !have.ephemeral.is_zero() || !requests.ephemeral.is_zero() || !limits.ephemeral.is_zero()
        {
            rows.push(make.row(
                AllocationKind::EphemeralStorage,
                (requests.ephemeral, limits.ephemeral),
                Caps::node(have.ephemeral),
                Usage::NotReported,
                ephemeral_unset(unset),
            ));
        }
        for name in gpu_names([have, requests, limits]) {
            let offered = Caps::node(count(have.gpu_count(&name)));
            rows.push(make.gpu(&name, requests, limits, offered));
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
        let make = Maker(CapacityKind::Quota);
        let quota =
            |requests: Option<Quantity>, limits: Option<Quantity>| Caps { requests, limits };
        let mut rows = vec![
            make.row(
                AllocationKind::Cpu,
                (requests.cpu, limits.cpu),
                quota(caps.requests.cpu, caps.limits.cpu),
                namespace_usage_of(usage, *pods, true),
                cpu_unset(unset),
            ),
            make.row(
                AllocationKind::Memory,
                (requests.memory, limits.memory),
                quota(caps.requests.memory, caps.limits.memory),
                namespace_usage_of(usage, *pods, false),
                memory_unset(unset),
            ),
        ];
        let ephemeral_used = !requests.ephemeral.is_zero() || !limits.ephemeral.is_zero();
        if ephemeral_used || caps.requests.ephemeral.is_some() || caps.limits.ephemeral.is_some() {
            rows.push(make.row(
                AllocationKind::EphemeralStorage,
                (requests.ephemeral, limits.ephemeral),
                quota(caps.requests.ephemeral, caps.limits.ephemeral),
                Usage::NotReported,
                ephemeral_unset(unset),
            ));
        }
        if caps.pods.is_some() || *pods > 0 {
            rows.push(make.pods(*pods, caps.pods));
        }
        // A quota can only cap GPU requests.
        let mut names = gpu_names([requests, limits]);
        names.extend(caps.requests.gpu.keys().cloned());
        names.sort();
        names.dedup();
        for name in names {
            let cap = caps.requests.gpu.get(&name).map(|n| count(*n));
            rows.push(make.gpu(&name, requests, limits, quota(cap, None)));
        }
        let mut notes = Vec::new();
        if let UsageState::Ready(measured) = usage
            && measured.pods_unreadable > 0
        {
            notes.push(format!(
                "usage leaves out {} {} with no reading yet",
                measured.pods_unreadable,
                plural(measured.pods_unreadable, "pod", "pods"),
            ));
        }
        if scoped_quotas > 0 {
            let (quota, caps, is) = if scoped_quotas == 1 {
                ("quota", "caps", "is")
            } else {
                ("quotas", "cap", "are")
            };
            notes.push(format!(
                "{scoped_quotas} scoped {quota} {caps} only some pods and {is} not drawn as a bar"
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

/// Builds the rows of one view, all with the same kind of capacity.
struct Maker(CapacityKind);

impl Maker {
    /// A row of CPU, memory or ephemeral storage: `(requests, limits)` against `caps`.
    fn row(
        &self,
        kind: AllocationKind,
        (requests, limits): (Quantity, Quantity),
        caps: Caps,
        usage: Usage,
        unset: UnsetCount,
    ) -> AllocationRow {
        AllocationRow {
            kind,
            capacity_kind: self.0,
            capacity: caps.requests,
            limits_capacity: caps.limits,
            requests,
            limits: Some(limits),
            usage,
            unset,
        }
    }

    /// The pod count against `max` (a node's `status.allocatable.pods`, a quota's `pods`).
    fn pods(&self, pods: u32, max: Option<u64>) -> AllocationRow {
        AllocationRow {
            kind: AllocationKind::Pods,
            capacity_kind: self.0,
            capacity: max.map(count),
            limits_capacity: None,
            requests: Quantity::from_value(i64::from(pods)),
            limits: None,
            usage: Usage::NotApplicable,
            unset: UnsetCount::default(),
        }
    }

    /// A GPU resource: a whole number, not measured by metrics-server.
    fn gpu(
        &self,
        name: &Arc<str>,
        requests: &ResourceAmounts,
        limits: &ResourceAmounts,
        caps: Caps,
    ) -> AllocationRow {
        self.row(
            AllocationKind::Gpu(name.clone()),
            (
                count(requests.gpu_count(name)),
                count(limits.gpu_count(name)),
            ),
            caps,
            Usage::NotReported,
            UnsetCount::default(),
        )
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

/// GPU resource names any of `amounts` mentions, sorted.
fn gpu_names<'a>(amounts: impl IntoIterator<Item = &'a ResourceAmounts>) -> Vec<Arc<str>> {
    let mut names: Vec<Arc<str>> = amounts
        .into_iter()
        .flat_map(|a| a.gpu.keys().cloned())
        .collect();
    names.sort();
    names.dedup();
    names
}

fn plural(n: u32, one: &'static str, many: &'static str) -> &'static str {
    if n == 1 { one } else { many }
}

fn count(n: u64) -> Quantity {
    Quantity::from_value(i64::try_from(n).unwrap_or(i64::MAX))
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
