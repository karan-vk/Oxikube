//! Reference index tests over `oxikube_testkit` fakes.
//!
//! * `index`: each reference kind, deltas (add, update, delete), status and scope
//! * `random`: random delta sequences equal a from-scratch rebuild
//! * `feed`: the index kept from a store's Pods feed, and the hub's lifecycle (the Pods feed is
//!   released when the last user goes)
//! * `workloads`: Pods grouped by the top of their owner chains

mod feed;
mod index;
mod random;
mod workloads;

use std::sync::Arc;

use oxikube_domain::Resource;
use serde_json::{Value, json};

use crate::references::ReferenceIndex;
use crate::store::{FeedState, RowChange, RowOp, StoreDelta, StoreObject};

pub(super) const CLUSTER: &str = "a";

/// A Pod `namespace/name` whose `spec` is `spec`, `Running`.
pub(super) fn pod_with(name: &str, namespace: &str, spec: Value) -> Resource {
    pod_in(name, namespace, spec, "Running")
}

/// [`pod_with`] with a `status.phase`.
pub(super) fn pod_in(name: &str, namespace: &str, spec: Value, phase: &str) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": {"name": name, "namespace": namespace, "uid": format!("uid-{namespace}-{name}")},
        "spec": spec,
        "status": {"phase": phase},
    }))
    .expect("a pod")
}

/// A Pod that mounts ConfigMap `cm` as a volume.
pub(super) fn pod_cm(name: &str, namespace: &str, cm: &str) -> Resource {
    pod_with(
        name,
        namespace,
        json!({"volumes": [{"name": "v", "configMap": {"name": cm}}]}),
    )
}

/// `pod` owned (as controller) by `kind` `owner` of `api_version`.
pub(super) fn owned_by(pod: Resource, api_version: &str, kind: &str, owner: &str) -> Resource {
    let mut json = pod.to_value();
    json["metadata"]["ownerReferences"] = json!([{
        "apiVersion": api_version, "kind": kind, "name": owner,
        "uid": format!("uid-{owner}"), "controller": true,
    }]);
    Resource::from_json(json).expect("still a resource")
}

pub(super) fn object(res: Resource) -> Arc<StoreObject> {
    Arc::new(StoreObject::Resource(res))
}

pub(super) fn delta(rows: RowChange, state: FeedState) -> StoreDelta {
    StoreDelta {
        rows,
        state,
        columns: None,
        len: 0,
        total: 0,
    }
}

pub(super) fn snapshot(pods: Vec<Resource>) -> StoreDelta {
    delta(
        RowChange::Snapshot(pods.into_iter().map(object).collect()),
        FeedState::Ready,
    )
}

pub(super) fn ops(ops: Vec<RowOp>) -> StoreDelta {
    delta(RowChange::Ops(ops), FeedState::Ready)
}

/// A ready index holding `pods`.
pub(super) fn ready(pods: Vec<Resource>) -> ReferenceIndex {
    let index = ReferenceIndex::new();
    index.apply(&snapshot(pods));
    index
}
