//! The workload kinds (E12-S04) in the views: their specs and sidebar places (`registry`), the
//! table (`columns`), the panel's data (`model`), the Scale action, key and dialog (`scale`), Restart
//! and Set image (`rollout`) and the detail panel (`panel`).
//! `#[gpui::test]`s over testkit fakes and the real bus and guard.

mod columns;
mod model;
mod panel;
mod registry;
mod rollout;
mod scale;

use oxikube_domain::Resource;
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_domain::kinds::{ResourceKind, VerbSet};
use serde_json::{Value, json};

use crate::table::tests::fixture::cluster;

/// A kind as discovery serves it.
pub(super) fn kind(group: &str, kind: &str, plural: &str) -> ResourceKind {
    ResourceKind {
        gvk: Gvk::new(group, "v1", kind),
        preferred: true,
        plural: plural.into(),
        singular: kind.to_lowercase(),
        short_names: Vec::new(),
        categories: Vec::new(),
        verbs: VerbSet::from_names(["get", "list", "watch", "delete"]),
        namespaced: true,
    }
}

pub(super) fn deployments() -> ResourceKind {
    kind("apps", "Deployment", "deployments")
}

pub(super) fn statefulsets() -> ResourceKind {
    kind("apps", "StatefulSet", "statefulsets")
}

pub(super) fn replicasets() -> ResourceKind {
    kind("apps", "ReplicaSet", "replicasets")
}

pub(super) fn daemonsets() -> ResourceKind {
    kind("apps", "DaemonSet", "daemonsets")
}

pub(super) fn replication_controllers() -> ResourceKind {
    kind("", "ReplicationController", "replicationcontrollers")
}

pub(super) fn pods() -> ResourceKind {
    kind("", "Pod", "pods")
}

fn template() -> Value {
    json!({
        "metadata": {"labels": {"app": "web"}},
        "spec": {"containers": [{
            "name": "app", "image": "shop/web:2.1",
            "ports": [{"name": "http", "containerPort": 8080}],
            "resources": {"requests": {"cpu": "100m"}, "limits": {"memory": "256Mi"}}
        }]}
    })
}

/// The Deployment `shop/web`: 3 wanted, 2 ready, rolling update, selector `app=web`.
pub(super) fn web_deployment() -> Resource {
    deployment_selecting("web", "10")
}

/// [`web_deployment`] selecting `app=<app>`, at resource version `rv`.
pub(super) fn deployment_selecting(app: &str, rv: &str) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "apps/v1", "kind": "Deployment",
        "metadata": {"name": "web", "namespace": "shop", "uid": "dep-uid", "resourceVersion": rv,
                     "creationTimestamp": "2026-01-01T00:00:00Z"},
        "spec": {
            "replicas": 3, "selector": {"matchLabels": {"app": app}},
            "strategy": {"type": "RollingUpdate",
                         "rollingUpdate": {"maxSurge": "25%", "maxUnavailable": 1}},
            "template": template()
        },
        "status": {"replicas": 3, "readyReplicas": 2, "updatedReplicas": 3, "availableReplicas": 2,
                   "conditions": [{"type": "Available", "status": "False",
                                   "reason": "MinimumReplicasUnavailable"}]}
    }))
    .expect("a deployment")
}

/// A ReplicaSet of `web`, owned by it (or by `owner` when given).
pub(super) fn web_replicaset(
    name: &str,
    uid: &str,
    desired: u32,
    ready: u32,
    owner: &str,
) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "apps/v1", "kind": "ReplicaSet",
        "metadata": {"name": name, "namespace": "shop", "uid": uid, "resourceVersion": "3",
                     "creationTimestamp": "2026-01-01T00:00:00Z",
                     "labels": {"app": "web"},
                     "ownerReferences": [{"apiVersion": "apps/v1", "kind": "Deployment",
                                          "name": "web", "uid": owner, "controller": true}]},
        "spec": {"replicas": desired, "selector": {"matchLabels": {"app": "web"}}},
        "status": {"replicas": desired, "readyReplicas": ready, "availableReplicas": ready}
    }))
    .expect("a replicaset")
}

/// A running pod of `web`, owned by the ReplicaSet `owner` (a uid), with `labels`.
pub(super) fn web_pod(name: &str, owner: &str, app: &str) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": {"name": name, "namespace": "shop", "uid": format!("{name}-uid"),
                     "resourceVersion": "1", "creationTimestamp": "2026-01-01T00:00:00Z",
                     "labels": {"app": app},
                     "ownerReferences": [{"apiVersion": "apps/v1", "kind": "ReplicaSet",
                                          "name": "rs", "uid": owner, "controller": true}]},
        "spec": {"nodeName": "n1", "containers": [{"name": "app", "image": "shop/web:2.1"}]},
        "status": {"phase": "Running", "containerStatuses": [
            {"name": "app", "ready": true, "restartCount": 0, "image": "shop/web:2.1",
             "state": {"running": {"startedAt": "2026-01-01T00:00:00Z"}}}]}
    }))
    .expect("a pod")
}

pub(super) fn web_ref() -> ResourceRef {
    ResourceRef::namespaced(
        cluster(),
        Gvk::new("apps", "v1", "Deployment"),
        "shop",
        "web",
    )
}
