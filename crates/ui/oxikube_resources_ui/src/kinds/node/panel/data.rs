//! What the panel draws, derived off the render path: the node's view-models, and the rows of
//! the pods on it.

use std::sync::Arc;

use oxikube_app::columns::Tone;
use oxikube_app::store::StoreObject;
use oxikube_domain::ids::{ClusterId, Gvk, ResourceRef};
use oxikube_domain::{
    AllocationResource, Demand, NodeDetail, NodeSummary, PodPlacement, PodSummary, Resource,
};

/// The node, as the panel shows it.
pub(super) struct NodeData {
    /// The `resourceVersion` it was built from: a new one rebuilds it.
    pub version: Option<Arc<str>>,
    pub summary: NodeSummary,
    pub detail: NodeDetail,
}

impl NodeData {
    /// `None` when `resource` is not a Node.
    pub fn of(resource: &Resource) -> Option<Self> {
        Some(Self {
            version: resource.meta.resource_version.clone(),
            summary: NodeSummary::from_resource(resource).ok()?,
            detail: NodeDetail::from_resource(resource),
        })
    }
}

/// One pod on the node.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct PodRow {
    pub namespace: Arc<str>,
    pub name: Arc<str>,
    /// `Running`, `CrashLoopBackOff`, ...
    pub status: Arc<str>,
    pub tone: Tone,
    /// CPU and memory requests, as text (`250m`, `128 MiB`).
    pub cpu: String,
    pub memory: String,
}

impl PodRow {
    pub fn of(pod: &Resource) -> Option<Self> {
        let summary = PodSummary::from_resource(pod).ok()?;
        let demand = PodPlacement::of_pod(pod)
            .map(|p| p.demand)
            .unwrap_or_default();
        let status = summary.status;
        Some(Self {
            namespace: pod.meta.namespace.clone()?,
            name: pod.meta.name.clone(),
            tone: oxikube_app::columns::status_tone(&status),
            status,
            cpu: AllocationResource::Cpu.format(demand.cpu_requests),
            memory: AllocationResource::Memory.format(demand.memory_requests),
        })
    }

    /// The pod as an object reference, for opening its detail.
    pub fn target(&self, cluster: &ClusterId) -> ResourceRef {
        ResourceRef::namespaced(
            cluster.clone(),
            Gvk::new("", "v1", "Pod"),
            &*self.namespace,
            &*self.name,
        )
    }
}

/// The pods on the node and what they take, with where the answer came from.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct PodsOnNode {
    pub demand: Demand,
    pub rows: Vec<PodRow>,
    /// Whether the store's live index answered (`false`: a one-time list).
    pub live: bool,
}

impl PodsOnNode {
    /// From the store's pods on a node (objects from the live Pod feed).
    pub fn from_objects(demand: Demand, pods: &[Arc<StoreObject>]) -> Self {
        Self {
            demand,
            rows: pods
                .iter()
                .filter_map(|o| o.resource())
                .filter_map(PodRow::of)
                .collect(),
            live: true,
        }
    }

    /// From a field-selector list.
    pub fn from_list(found: &oxikube_app::nodes::NodePods) -> Self {
        Self {
            demand: found.demand,
            rows: found.pods.iter().filter_map(|p| PodRow::of(p)).collect(),
            live: false,
        }
    }
}

/// What the rows were last built from: the demand and the very objects the live index handed
/// over.
///
/// It keeps the objects alive on purpose. An updated pod is a new `Arc`, so "the same objects"
/// means pointer equality, and that only holds while no address can be reused: the store frees an
/// object when it replaces it, and the allocator may hand the address to the next update, so
/// pointers kept without their objects could match by accident and hide a status change. A node
/// has a hundred pods or so; keeping their last versions costs nothing.
#[derive(Debug)]
pub(super) struct Followed {
    demand: Demand,
    pods: Vec<Arc<StoreObject>>,
}

impl Followed {
    pub fn of(demand: Demand, pods: &[Arc<StoreObject>]) -> Self {
        Self {
            demand,
            pods: pods.to_vec(),
        }
    }

    /// Whether `demand` and `pods` are exactly what the rows were built from: no pod added,
    /// removed or replaced by a newer version.
    pub fn is_current(&self, demand: &Demand, pods: &[Arc<StoreObject>]) -> bool {
        self.demand == *demand
            && self.pods.len() == pods.len()
            && self.pods.iter().zip(pods).all(|(a, b)| Arc::ptr_eq(a, b))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxikube_testkit::pod;

    fn stored(running: bool) -> Arc<StoreObject> {
        let builder = pod().namespace("x").name("p").node("n");
        let builder = if running {
            builder.running()
        } else {
            builder.container_creating()
        };
        Arc::new(StoreObject::Resource(builder.build()))
    }

    #[test]
    fn a_newer_version_of_a_pod_is_never_current_even_after_the_old_one_is_gone() {
        let before = stored(false);
        let followed = Followed::of(Demand::ZERO, std::slice::from_ref(&before));
        let weak = Arc::downgrade(&before);
        // The store replaces the pod and lets go of the old object...
        drop(before);
        assert!(
            weak.upgrade().is_some(),
            "the followed set keeps the old version alive, so its address cannot be reused"
        );
        // ...so a later version, whatever it is allocated next to, is a different object.
        let after = stored(true);
        assert!(!followed.is_current(&Demand::ZERO, std::slice::from_ref(&after)));
        assert!(!followed.is_current(&Demand::ZERO, &[]));
    }

    #[test]
    fn the_same_objects_and_demand_are_current() {
        let pods = vec![stored(true)];
        let followed = Followed::of(Demand::ZERO, &pods);
        assert!(followed.is_current(&Demand::ZERO, &pods));
        let other = Demand {
            pods: 1,
            ..Demand::ZERO
        };
        assert!(!followed.is_current(&other, &pods));
    }
}
