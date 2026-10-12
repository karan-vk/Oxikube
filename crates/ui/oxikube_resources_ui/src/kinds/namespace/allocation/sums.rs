//! The sums behind a namespace's allocation section, computed off the UI thread.

use std::sync::Arc;

use oxikube_app::allocate_namespaces;
use oxikube_app::store::StoreObject;
use oxikube_app::{NamespaceAllocation, allocation::AllocationView};
use oxikube_domain::Resource;
use oxikube_domain::utilisation::{GpuVendors, QuotaCaps, ResourceAmounts, UnsetPods};
use oxikube_domain::view::{QuotaLine, QuotaSummary};

/// One `ResourceQuota` of the namespace, as the server reports it.
#[derive(Debug, Clone, PartialEq)]
pub(in crate::kinds::namespace) struct QuotaView {
    pub name: Arc<str>,
    /// `spec.scopes` and the scope selector: a scoped quota caps only the pods it selects.
    pub scopes: Vec<String>,
    /// `used / hard` per resource, from the quota's status.
    pub lines: Vec<QuotaLine>,
}

/// What the pods and quotas of a namespace add up to.
#[derive(Debug, Clone, PartialEq)]
pub(in crate::kinds::namespace) struct Sums {
    pub alloc: NamespaceAllocation,
    pub caps: QuotaCaps,
    /// Quotas left out of `caps` because they cap only some pods.
    pub scoped: usize,
    pub quotas: Vec<QuotaView>,
}

impl Sums {
    /// The allocation section for these sums under `usage`.
    pub fn view(&self, usage: &oxikube_app::allocation::UsageState) -> AllocationView {
        AllocationView::for_namespace(&self.alloc, &self.caps, self.scoped, usage)
    }
}

/// Whether the pods can be totalled: every one is a full object (a feed the watch budget reduced
/// to metadata, or a Table feed, holds no `spec`, so its sums would be silently low).
pub(in crate::kinds::namespace) fn totals_possible(pods: &[Arc<StoreObject>]) -> bool {
    pods.iter()
        .all(|o| o.resource().is_some_and(|r| !r.is_partial()))
}

/// Sums `pods` (the pods of `namespace`) with S02's [`allocate_namespaces`] and reads `quotas`.
pub(in crate::kinds::namespace) fn compute(
    namespace: &Arc<str>,
    pods: &[Arc<StoreObject>],
    quotas: &[Arc<StoreObject>],
    gpus: &GpuVendors,
) -> Sums {
    let pods = pods.iter().filter_map(|o| o.resource());
    let alloc = allocate_namespaces(pods, gpus)
        .into_iter()
        .find(|a| a.namespace == *namespace)
        .unwrap_or_else(|| NamespaceAllocation {
            namespace: namespace.clone(),
            pods: 0,
            requests: ResourceAmounts::zero(),
            limits: ResourceAmounts::zero(),
            unset: UnsetPods::default(),
        });
    let quotas: Vec<&Resource> = quotas.iter().filter_map(|o| o.resource()).collect();
    let (caps, scoped) = QuotaCaps::tightest(quotas.iter().copied(), gpus);
    let mut views: Vec<QuotaView> = quotas
        .iter()
        .filter_map(|res| {
            let summary = QuotaSummary::from_resource(res).ok()?;
            let scopes = summary
                .scopes
                .iter()
                .map(|s| s.to_string())
                .chain(summary.scope_selector.iter().cloned())
                .collect();
            Some(QuotaView {
                name: res.meta.name.clone(),
                scopes,
                lines: summary.lines,
            })
        })
        .collect();
    views.sort_by(|a, b| a.name.cmp(&b.name));
    Sums {
        alloc,
        caps,
        scoped,
        quotas: views,
    }
}
