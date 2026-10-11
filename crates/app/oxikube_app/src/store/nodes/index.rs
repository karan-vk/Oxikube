//! [`PodsByNode`]: pods grouped by `spec.nodeName`, with the summed [`Demand`] of each group.

use std::collections::HashMap;
use std::sync::Arc;

use oxikube_domain::{Demand, PodPlacement};

use crate::store::object::{ObjectKey, StoreObject};

/// Where one pod is counted.
#[derive(Debug)]
struct Placed {
    node: Arc<str>,
    demand: Demand,
}

/// One node's pods.
#[derive(Debug, Default)]
struct Bucket {
    demand: Demand,
    pods: HashMap<ObjectKey, Arc<StoreObject>>,
}

/// The index. Built from a cache's objects, then kept in step by [`upsert`](Self::upsert) and
/// [`remove`](Self::remove) per delta.
#[derive(Debug, Default)]
pub(crate) struct PodsByNode {
    placed: HashMap<ObjectKey, Placed>,
    nodes: HashMap<Arc<str>, Bucket>,
    /// Counts the changes to what a node holds (a pod arrived, left, moved, or asks for another
    /// amount); a pod whose status changes does not move it. A sort by a node's load is stale
    /// exactly when this moved.
    version: u64,
}

impl PodsByNode {
    /// An index over `objects`.
    pub fn build<'a>(objects: impl Iterator<Item = &'a Arc<StoreObject>>) -> Self {
        let mut index = Self::default();
        for object in objects {
            index.upsert(&object.key(), object);
        }
        index
    }

    /// Counts `object` (a pod) under its node, replacing what it contributed before. A pod that
    /// is not on a node, or has finished, is not counted.
    pub fn upsert(&mut self, key: &ObjectKey, object: &Arc<StoreObject>) {
        let placement = object.resource().and_then(PodPlacement::of_pod);
        if let (Some(now), Some(before)) = (&placement, self.placed.get(key))
            && before.node == now.node
            && before.demand == now.demand
        {
            // Same node, same amounts (a status change): only the stored object is new.
            if let Some(slot) = self
                .nodes
                .get_mut(&before.node)
                .and_then(|bucket| bucket.pods.get_mut(key))
            {
                *slot = object.clone();
            }
            return;
        }
        self.remove(key);
        let Some(placement) = placement else {
            return;
        };
        let bucket = self.nodes.entry(placement.node.clone()).or_default();
        bucket.demand += placement.demand;
        bucket.pods.insert(key.clone(), object.clone());
        self.placed.insert(
            key.clone(),
            Placed {
                node: placement.node,
                demand: placement.demand,
            },
        );
        self.version += 1;
    }

    /// Stops counting the pod at `key`.
    pub fn remove(&mut self, key: &ObjectKey) {
        let Some(placed) = self.placed.remove(key) else {
            return;
        };
        self.version += 1;
        if let Some(bucket) = self.nodes.get_mut(&placed.node) {
            bucket.demand -= placed.demand;
            bucket.pods.remove(key);
            if bucket.pods.is_empty() {
                self.nodes.remove(&placed.node);
            }
        }
    }

    /// How many times what a node holds has changed (see the field).
    pub fn version(&self) -> u64 {
        self.version
    }

    /// The summed demand of the pods on `node`; zero for a node with none.
    pub fn demand(&self, node: &str) -> Demand {
        self.nodes.get(node).map_or(Demand::ZERO, |b| b.demand)
    }

    /// The pods on `node`, ordered by namespace and name.
    pub fn pods(&self, node: &str) -> Vec<Arc<StoreObject>> {
        let Some(bucket) = self.nodes.get(node) else {
            return Vec::new();
        };
        let mut pods: Vec<_> = bucket.pods.iter().collect();
        pods.sort_unstable_by(|a, b| a.0.cmp(b.0));
        pods.into_iter().map(|(_, o)| o.clone()).collect()
    }

    /// Pods counted.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.placed.len()
    }
}
