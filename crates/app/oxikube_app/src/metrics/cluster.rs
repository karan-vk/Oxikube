//! Cluster-wide CPU and memory use (E13-S10): what the nodes report against what they offer.
//!
//! [`cluster_utilisation`] sums the latest metrics-server reading of every node that has one and
//! compares it with the same nodes' `status.allocatable`, the maths of the node table's `CPU%`
//! and `Mem%` columns taken over the whole cluster. Its history is built from the per-node rings
//! the polling service already keeps ([`ClusterUsage::node_history`]), newest reading first, so
//! a view that opens late still has a sparkline and nothing has to sample on its own.
//!
//! A node without a reading counts for neither side: a cluster of ten nodes with seven reporting
//! is seven nodes' use over seven nodes' allocatable, never ten nodes' allocatable over seven
//! nodes' use (which would read low). [`ClusterUtilisation::nodes_reporting`] says how many.
//!
//! Pure over its inputs and allocation-light, but linear in nodes times ring length, so a caller
//! with many nodes runs it off the UI thread.

use jiff::Timestamp;
use oxikube_domain::Resource;
use oxikube_domain::utilisation::{GpuVendors, NodeAllocatable};

use super::live::{ClusterUsage, UsagePoint};

/// One resource's cluster-wide figure.
#[derive(Debug, Clone, PartialEq)]
pub struct ResourceUse {
    /// The latest readings summed: nanocores for CPU, bytes for memory.
    pub used: u128,
    /// The reporting nodes' allocatable: nanocores for CPU, bytes for memory.
    pub allocatable: u128,
    /// `used` as a percent of `allocatable`; `None` when the nodes offer none (a dash, never 0).
    pub percent: Option<f64>,
    /// The percent over time, oldest first (at most one point per ring slot).
    pub history: Vec<(Timestamp, f64)>,
}

/// The cluster's CPU and memory use. See the [module docs](self).
#[derive(Debug, Clone, PartialEq)]
pub struct ClusterUtilisation {
    /// Nodes in the cluster.
    pub nodes: usize,
    /// Nodes with a reading, the ones the figures are over.
    pub nodes_reporting: usize,
    /// CPU.
    pub cpu: ResourceUse,
    /// Memory.
    pub memory: ResourceUse,
}

/// One node's allocatable and its readings.
struct Reporting {
    cpu_nanocores: u128,
    memory_bytes: u128,
    history: Vec<UsagePoint>,
}

/// The cluster's use over `nodes`, from `usage`; `None` when no node has a reading yet (the
/// caller shows the metrics state instead).
pub fn cluster_utilisation<'a>(
    usage: &ClusterUsage,
    nodes: impl IntoIterator<Item = &'a Resource>,
    gpus: &GpuVendors,
) -> Option<ClusterUtilisation> {
    let mut total = 0;
    let mut reporting = Vec::new();
    for node in nodes {
        let Ok(node) = NodeAllocatable::from_resource(node, gpus) else {
            continue;
        };
        total += 1;
        let history = usage.node_history(&node.name);
        if history.is_empty() {
            continue;
        }
        reporting.push(Reporting {
            cpu_nanocores: u128::try_from(node.allocatable.cpu.milli_value()).unwrap_or(0)
                * 1_000_000,
            memory_bytes: u128::try_from(node.allocatable.memory.value()).unwrap_or(0),
            history,
        });
    }
    if reporting.is_empty() {
        return None;
    }
    Some(ClusterUtilisation {
        nodes: total,
        nodes_reporting: reporting.len(),
        cpu: resource_use(&reporting, |n| n.cpu_nanocores, |p| p.cpu_nanocores),
        memory: resource_use(&reporting, |n| n.memory_bytes, |p| p.memory_bytes),
    })
}

/// One resource over `nodes`: `offered` is a node's allocatable, `read` a point's value.
fn resource_use(
    nodes: &[Reporting],
    offered: impl Fn(&Reporting) -> u128,
    read: impl Fn(&UsagePoint) -> Option<u64>,
) -> ResourceUse {
    let depth = nodes.iter().map(|n| n.history.len()).max().unwrap_or(0);
    let mut history = Vec::with_capacity(depth);
    let mut latest = (0u128, 0u128);
    // Step back from each node's newest reading: the k-th newest of every node was taken in the
    // same poll, so they add up. A node that joined later has fewer steps and drops out of the
    // older sums, and its allocatable with it.
    for back in 0..depth {
        let (mut used, mut allocatable) = (0u128, 0u128);
        let mut at: Option<Timestamp> = None;
        for node in nodes {
            let Some(point) = node
                .history
                .len()
                .checked_sub(back + 1)
                .map(|i| node.history[i])
            else {
                continue;
            };
            let Some(value) = read(&point) else {
                continue;
            };
            used += u128::from(value);
            allocatable += offered(node);
            at = Some(at.map_or(point.at, |t| t.max(point.at)));
        }
        if back == 0 {
            latest = (used, allocatable);
        }
        if let (Some(at), Some(percent)) = (at, percent(used, allocatable)) {
            history.push((at, percent));
        }
    }
    history.reverse();
    ResourceUse {
        used: latest.0,
        allocatable: latest.1,
        percent: percent(latest.0, latest.1),
        history,
    }
}

/// `used` over `allocatable` in percent; `None` over nothing.
fn percent(used: u128, allocatable: u128) -> Option<f64> {
    (allocatable > 0).then(|| used as f64 * 100.0 / allocatable as f64)
}
