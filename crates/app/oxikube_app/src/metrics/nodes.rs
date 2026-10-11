//! Node allocation: what each node offers against what the pods on it ask for.

use std::collections::HashMap;
use std::sync::Arc;

use oxikube_domain::Resource;
use oxikube_domain::utilisation::{
    AmountPercents, GpuVendors, NodeAllocatable, PodAllocation, ResourceAmounts,
};

/// One node's allocation, as `kubectl describe node` shows under "Allocated resources".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeAllocation {
    /// The node.
    pub node: NodeAllocatable,
    /// Sum of the effective requests of the pods on the node.
    pub requests: ResourceAmounts,
    /// Sum of the effective limits of the pods on the node. Limits may exceed allocatable
    /// (overcommit); the percent is then above 100.
    pub limits: ResourceAmounts,
    /// Pods on the node that still hold resources.
    pub pods: u32,
}

impl NodeAllocation {
    /// Requests as a percent of allocatable (`None` per resource the node does not offer).
    pub fn requests_percent(&self) -> AmountPercents {
        self.requests.percent_of(&self.node.allocatable)
    }

    /// Limits as a percent of allocatable.
    pub fn limits_percent(&self) -> AmountPercents {
        self.limits.percent_of(&self.node.allocatable)
    }

    /// Pods as a percent of the node's pod capacity; `None` when it does not report one.
    pub fn pods_percent(&self) -> Option<f64> {
        let max = self.node.max_pods.filter(|&m| m > 0)?;
        Some(f64::from(self.pods) * 100.0 / max as f64)
    }
}

/// Allocation per node, in the order of `nodes`.
///
/// Each pod that still holds resources and is scheduled on one of `nodes` adds to that node;
/// pods on a node that is not in the list, unscheduled pods and terminated pods add to none.
/// Entries of either list that are not a Node / Pod are skipped.
pub fn allocate_nodes<'a>(
    nodes: impl IntoIterator<Item = &'a Resource>,
    pods: impl IntoIterator<Item = &'a Resource>,
    gpus: &GpuVendors,
) -> Vec<NodeAllocation> {
    let mut out: Vec<NodeAllocation> = nodes
        .into_iter()
        .filter_map(|node| NodeAllocatable::from_resource(node, gpus).ok())
        .map(|node| NodeAllocation {
            node,
            requests: ResourceAmounts::zero(),
            limits: ResourceAmounts::zero(),
            pods: 0,
        })
        .collect();
    let index: HashMap<Arc<str>, usize> = out
        .iter()
        .enumerate()
        .map(|(i, a)| (a.node.name.clone(), i))
        .collect();
    for pod in pods {
        let Ok(pod) = PodAllocation::from_resource(pod, gpus) else {
            continue;
        };
        if !pod.holds_resources() {
            continue;
        }
        let Some(&slot) = pod.node.as_ref().and_then(|node| index.get(node)) else {
            continue;
        };
        let target = &mut out[slot];
        target.requests += &pod.resources.requests;
        target.limits += &pod.resources.limits;
        target.pods += 1;
    }
    out
}
