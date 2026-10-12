//! Namespace aggregation: the sum a `ResourceQuota` would compare against.

use std::collections::BTreeMap;
use std::sync::Arc;

use oxikube_domain::Resource;
use oxikube_domain::utilisation::{
    AmountPercents, GpuVendors, PodAllocation, ResourceAmounts, UnsetPods,
};

/// One namespace's totals over the pods that still hold resources.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamespaceAllocation {
    /// The namespace.
    pub namespace: Arc<str>,
    /// Pods counted (`Pending` ones included, as a quota counts them).
    pub pods: u32,
    /// Sum of effective requests (compare with a quota's `requests.*` hard amounts).
    pub requests: ResourceAmounts,
    /// Sum of effective limits (compare with a quota's `limits.*` hard amounts).
    pub limits: ResourceAmounts,
    /// How many of the pods set no request or limit, per resource (E13-S11).
    pub unset: UnsetPods,
}

impl NamespaceAllocation {
    /// Requests as a percent of a quota's hard requests (`None` per resource the quota does not
    /// cap). The input of the allocation views (E13-S11), which parse the quota with
    /// [`QuotaCaps`](oxikube_domain::utilisation::QuotaCaps).
    pub fn requests_vs(&self, hard: &ResourceAmounts) -> AmountPercents {
        self.requests.percent_of(hard)
    }

    /// Limits as a percent of a quota's hard limits.
    pub fn limits_vs(&self, hard: &ResourceAmounts) -> AmountPercents {
        self.limits.percent_of(hard)
    }
}

/// Totals per namespace, sorted by name. A namespace appears once it has at least one pod that
/// holds resources. Pods without a namespace and entries that are not pods are skipped.
pub fn allocate_namespaces<'a>(
    pods: impl IntoIterator<Item = &'a Resource>,
    gpus: &GpuVendors,
) -> Vec<NamespaceAllocation> {
    let mut by_namespace: BTreeMap<Arc<str>, NamespaceAllocation> = BTreeMap::new();
    for pod in pods {
        let Ok(pod) = PodAllocation::from_resource(pod, gpus) else {
            continue;
        };
        let (true, Some(namespace)) = (pod.holds_resources(), pod.namespace) else {
            continue;
        };
        let entry = by_namespace
            .entry(namespace.clone())
            .or_insert_with(|| NamespaceAllocation {
                namespace,
                pods: 0,
                requests: ResourceAmounts::zero(),
                limits: ResourceAmounts::zero(),
                unset: UnsetPods::default(),
            });
        entry.requests += &pod.resources.requests;
        entry.limits += &pod.resources.limits;
        entry.pods += 1;
        entry.unset.add(&pod.resources);
    }
    by_namespace.into_values().collect()
}
