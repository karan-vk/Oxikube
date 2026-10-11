//! [`UtilisationAggregator`]: the one entry point for utilisation figures.

use oxikube_domain::Resource;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::metrics::MetricsSample;
use oxikube_domain::utilisation::{PodResources, Utilisation, aggregate_pod_requests_limits};
use oxikube_domain::view::ViewError;

use super::{GpuVendorsHandle, NamespaceAllocation, NodeAllocation};
use super::{allocate_namespaces, allocate_nodes};

/// Computes pod, node and namespace figures with the GPU vendor list of the cluster as it is
/// when the call is made, so a change of `metrics.gpu_vendors` is picked up by the next
/// computation. Cheap to clone; clones share the [`GpuVendorsHandle`].
#[derive(Debug, Clone, Default)]
pub struct UtilisationAggregator {
    vendors: GpuVendorsHandle,
}

impl UtilisationAggregator {
    /// An aggregator reading `vendors`.
    pub fn new(vendors: GpuVendorsHandle) -> Self {
        Self { vendors }
    }

    /// The handle the aggregator reads (the binary keeps one to push settings into).
    pub fn vendors(&self) -> &GpuVendorsHandle {
        &self.vendors
    }

    /// A pod's effective requests and limits.
    ///
    /// # Errors
    ///
    /// [`ViewError::WrongKind`] when `pod` is not a Pod.
    pub fn pod_resources(
        &self,
        cluster: &ClusterId,
        pod: &Resource,
    ) -> Result<PodResources, ViewError> {
        aggregate_pod_requests_limits(pod, &self.vendors.for_cluster(cluster))
    }

    /// The four pod-table percents for a pod and its latest metrics sample; dashes (`None`)
    /// where the sample or the request / limit is missing.
    ///
    /// # Errors
    ///
    /// [`ViewError::WrongKind`] when `pod` is not a Pod.
    pub fn pod_utilisation(
        &self,
        cluster: &ClusterId,
        pod: &Resource,
        sample: &MetricsSample,
    ) -> Result<Utilisation, ViewError> {
        let resources = self.pod_resources(cluster, pod)?;
        Ok(Utilisation::of_sample(sample, &resources))
    }

    /// Allocation per node (see [`allocate_nodes`]).
    pub fn nodes<'a>(
        &self,
        cluster: &ClusterId,
        nodes: impl IntoIterator<Item = &'a Resource>,
        pods: impl IntoIterator<Item = &'a Resource>,
    ) -> Vec<NodeAllocation> {
        allocate_nodes(nodes, pods, &self.vendors.for_cluster(cluster))
    }

    /// Totals per namespace (see [`allocate_namespaces`]).
    pub fn namespaces<'a>(
        &self,
        cluster: &ClusterId,
        pods: impl IntoIterator<Item = &'a Resource>,
    ) -> Vec<NamespaceAllocation> {
        allocate_namespaces(pods, &self.vendors.for_cluster(cluster))
    }
}
