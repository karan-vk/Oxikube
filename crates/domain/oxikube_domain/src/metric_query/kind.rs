//! [`MetricScope`], [`MetricKind`] and [`MetricUnit`], with the support matrix between them.

use std::fmt;

/// The level of the cluster a graph is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MetricScope {
    /// The whole cluster (every node).
    Cluster,
    /// One node.
    Node,
    /// One namespace.
    Namespace,
    /// One pod, or a set of pods (a workload's); one series per pod.
    Pod,
    /// The containers of one pod; one series per container.
    Container,
    /// One PersistentVolumeClaim.
    Pvc,
    /// One Ingress.
    Ingress,
}

impl MetricScope {
    /// Every scope, widest first.
    pub const ALL: [MetricScope; 7] = [
        MetricScope::Cluster,
        MetricScope::Node,
        MetricScope::Namespace,
        MetricScope::Pod,
        MetricScope::Container,
        MetricScope::Pvc,
        MetricScope::Ingress,
    ];

    /// A stable lower-case id (`pvc`), used in tool arguments and golden files.
    pub const fn id(self) -> &'static str {
        match self {
            MetricScope::Cluster => "cluster",
            MetricScope::Node => "node",
            MetricScope::Namespace => "namespace",
            MetricScope::Pod => "pod",
            MetricScope::Container => "container",
            MetricScope::Pvc => "pvc",
            MetricScope::Ingress => "ingress",
        }
    }

    /// Whether a query exists for `kind` at this scope. The answer does not depend on the
    /// provider: a PVC has disk but no CPU, an ingress has traffic only, a container has no
    /// network of its own (cAdvisor counts it per pod), and a pod count of one pod is one.
    pub const fn supports(self, kind: MetricKind) -> bool {
        use MetricKind::{DiskCapacity, DiskUsage, NetworkRx, NetworkTx, PodCount};
        match self {
            MetricScope::Cluster | MetricScope::Node | MetricScope::Namespace => true,
            MetricScope::Pod => !matches!(kind, DiskCapacity | PodCount),
            MetricScope::Container => {
                !matches!(kind, NetworkRx | NetworkTx | DiskCapacity | PodCount)
            }
            MetricScope::Pvc => matches!(kind, DiskUsage | DiskCapacity),
            MetricScope::Ingress => matches!(kind, NetworkRx | NetworkTx),
        }
    }
}

impl fmt::Display for MetricScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id())
    }
}

/// What a graph measures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MetricKind {
    /// CPU in use, in cores.
    CpuUsage,
    /// CPU requested by containers, in cores.
    CpuRequests,
    /// CPU limits of containers, in cores.
    CpuLimits,
    /// Memory in use (working set), in bytes.
    MemoryUsage,
    /// Memory requested by containers, in bytes.
    MemoryRequests,
    /// Memory limits of containers, in bytes.
    MemoryLimits,
    /// Bytes received per second.
    NetworkRx,
    /// Bytes sent per second.
    NetworkTx,
    /// Disk in use, in bytes (node filesystem, PVC, container writable layer).
    DiskUsage,
    /// Disk size, in bytes (node filesystem, PVC).
    DiskCapacity,
    /// Pods that are pending or running.
    PodCount,
}

impl MetricKind {
    /// Every kind, in the order a picker lists them.
    pub const ALL: [MetricKind; 11] = [
        MetricKind::CpuUsage,
        MetricKind::CpuRequests,
        MetricKind::CpuLimits,
        MetricKind::MemoryUsage,
        MetricKind::MemoryRequests,
        MetricKind::MemoryLimits,
        MetricKind::NetworkRx,
        MetricKind::NetworkTx,
        MetricKind::DiskUsage,
        MetricKind::DiskCapacity,
        MetricKind::PodCount,
    ];

    /// A stable lower-case id (`cpu_usage`), used in tool arguments and golden files.
    pub const fn id(self) -> &'static str {
        match self {
            MetricKind::CpuUsage => "cpu_usage",
            MetricKind::CpuRequests => "cpu_requests",
            MetricKind::CpuLimits => "cpu_limits",
            MetricKind::MemoryUsage => "memory_usage",
            MetricKind::MemoryRequests => "memory_requests",
            MetricKind::MemoryLimits => "memory_limits",
            MetricKind::NetworkRx => "network_rx",
            MetricKind::NetworkTx => "network_tx",
            MetricKind::DiskUsage => "disk_usage",
            MetricKind::DiskCapacity => "disk_capacity",
            MetricKind::PodCount => "pod_count",
        }
    }

    /// The unit of the values a query of this kind returns.
    pub const fn unit(self) -> MetricUnit {
        match self {
            MetricKind::CpuUsage | MetricKind::CpuRequests | MetricKind::CpuLimits => {
                MetricUnit::Cores
            }
            MetricKind::MemoryUsage
            | MetricKind::MemoryRequests
            | MetricKind::MemoryLimits
            | MetricKind::DiskUsage
            | MetricKind::DiskCapacity => MetricUnit::Bytes,
            MetricKind::NetworkRx | MetricKind::NetworkTx => MetricUnit::BytesPerSecond,
            MetricKind::PodCount => MetricUnit::Count,
        }
    }

    /// Whether the series is a reference line (a request, a limit or a capacity) rather than
    /// a measurement: views draw these as thresholds next to the usage series.
    pub const fn is_reference(self) -> bool {
        matches!(
            self,
            MetricKind::CpuRequests
                | MetricKind::CpuLimits
                | MetricKind::MemoryRequests
                | MetricKind::MemoryLimits
                | MetricKind::DiskCapacity
        )
    }
}

impl fmt::Display for MetricKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id())
    }
}

/// The unit of a query's values, for the views' formatters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MetricUnit {
    /// CPU cores (`0.25` is 250m).
    Cores,
    /// Bytes.
    Bytes,
    /// Bytes per second.
    BytesPerSecond,
    /// A plain count.
    Count,
}
