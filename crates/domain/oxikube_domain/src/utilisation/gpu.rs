//! [`GpuVendors`]: which extended resources count as GPUs (`metrics.gpu_vendors`).

use std::collections::HashMap;
use std::sync::Arc;

use crate::ids::ClusterId;

/// The extended resource names counted as GPUs when `metrics.gpu_vendors` says nothing else.
///
/// `default.json` carries the same list; a test in `oxikube_settings` keeps them equal.
pub const DEFAULT_GPU_VENDORS: [&str; 4] = [
    "nvidia.com/gpu",
    "amd.com/gpu",
    "gpu.intel.com/i915",
    "gpu.intel.com/xe",
];

/// The extended resource names that count as GPUs, in the order the user wrote them.
///
/// Names are exact Kubernetes resource names (`nvidia.com/gpu`; for a MIG profile
/// `nvidia.com/mig-1g.5gb`). Blank entries and repeats are dropped on construction. Cheap to
/// clone: the list is shared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuVendors {
    names: Arc<[Arc<str>]>,
}

impl GpuVendors {
    /// The list of `names`, trimmed, without blanks or repeats (the first spelling counts).
    pub fn new<I, S>(names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut out: Vec<Arc<str>> = Vec::new();
        for name in names {
            let name = name.as_ref().trim();
            if !name.is_empty() && !out.iter().any(|seen| &**seen == name) {
                out.push(Arc::from(name));
            }
        }
        Self { names: out.into() }
    }

    /// No GPU resource is counted.
    pub fn none() -> Self {
        Self::new::<[&str; 0], &str>([])
    }

    /// The resource names, in order.
    pub fn names(&self) -> &[Arc<str>] {
        &self.names
    }

    /// Whether `resource` is counted as a GPU.
    pub fn contains(&self, resource: &str) -> bool {
        self.names.iter().any(|name| &**name == resource)
    }

    /// Whether no GPU resource is counted.
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
}

impl Default for GpuVendors {
    /// [`DEFAULT_GPU_VENDORS`].
    fn default() -> Self {
        Self::new(DEFAULT_GPU_VENDORS)
    }
}

/// The GPU vendor list of every cluster: a default plus the clusters that override it
/// (`clusters.<id>.metrics.gpu_vendors`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GpuVendorsTable {
    default: GpuVendors,
    by_cluster: HashMap<ClusterId, GpuVendors>,
}

impl GpuVendorsTable {
    /// A table where every cluster uses `default`.
    pub fn new(default: GpuVendors) -> Self {
        Self {
            default,
            by_cluster: HashMap::new(),
        }
    }

    /// The same table with `cluster` using `vendors`.
    #[must_use]
    pub fn with_cluster(mut self, cluster: ClusterId, vendors: GpuVendors) -> Self {
        if vendors != self.default {
            self.by_cluster.insert(cluster, vendors);
        }
        self
    }

    /// The list for `cluster`: its own, else the default.
    pub fn for_cluster(&self, cluster: &ClusterId) -> &GpuVendors {
        self.by_cluster.get(cluster).unwrap_or(&self.default)
    }

    /// The list for clusters with no override.
    pub fn default_vendors(&self) -> &GpuVendors {
        &self.default
    }
}
