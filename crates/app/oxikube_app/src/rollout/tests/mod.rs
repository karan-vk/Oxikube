//! Tests of the rollout history and `workload::Rollback` over the real guard and the testkit
//! fakes. Fixtures: a three-revision Deployment `shop/web` with change causes (revision 3 is
//! current), and a StatefulSet `shop/db` with three ControllerRevisions, both in cluster `a`.

mod guard;
mod history;
mod rollback;
mod runner;

use oxikube_domain::Resource;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_testkit::{FakeResourcePort, ResourceCall};
use serde_json::{Value, json};
use std::sync::Arc;

use crate::testing::{Harness, id};

fn object(value: Value) -> Resource {
    Resource::from_json(value).expect("a valid object")
}

fn template(app: &str, image: &str, hash: Option<&str>) -> Value {
    let mut labels = json!({"app": app});
    if let Some(hash) = hash {
        labels["pod-template-hash"] = json!(hash);
    }
    json!({
        "metadata": {"labels": labels, "creationTimestamp": null},
        "spec": {"containers": [{"name": app, "image": image}]},
    })
}

/// Deployment `shop/web` at `revision` running `image`.
pub(super) fn deployment_object(revision: i64, image: &str, paused: bool) -> Resource {
    object(json!({
        "apiVersion": "apps/v1", "kind": "Deployment",
        "metadata": {
            "name": "web", "namespace": "shop", "uid": "web-uid",
            "annotations": {"deployment.kubernetes.io/revision": revision.to_string()},
        },
        "spec": {"paused": paused, "selector": {"matchLabels": {"app": "web"}},
                 "template": template("web", image, None)},
    }))
}

fn replicaset(name: &str, revision: i64, image: &str, cause: Option<&str>) -> Resource {
    let mut annotations = json!({"deployment.kubernetes.io/revision": revision.to_string()});
    if let Some(cause) = cause {
        annotations["kubernetes.io/change-cause"] = json!(cause);
    }
    object(json!({
        "apiVersion": "apps/v1", "kind": "ReplicaSet",
        "metadata": {
            "name": name, "namespace": "shop", "uid": format!("{name}-uid"),
            "creationTimestamp": "2026-10-01T10:00:00Z",
            "labels": {"app": "web", "pod-template-hash": "abc"},
            "annotations": annotations,
            "ownerReferences": [{"apiVersion": "apps/v1", "kind": "Deployment", "name": "web",
                                 "uid": "web-uid", "controller": true}],
        },
        "spec": {"replicas": if revision == 3 { 2 } else { 0 },
                 "template": template("web", image, Some("abc"))},
        "status": {"readyReplicas": if revision == 3 { 2 } else { 0 }},
    }))
}

fn controller_revision(revision: i64, image: &str) -> Resource {
    let mut stored = template("db", image, None);
    stored["$patch"] = json!("replace");
    object(json!({
        "apiVersion": "apps/v1", "kind": "ControllerRevision",
        "metadata": {
            "name": format!("db-{revision}"), "namespace": "shop",
            "creationTimestamp": "2026-10-01T10:00:00Z",
            "labels": {"app": "db"},
            "ownerReferences": [{"apiVersion": "apps/v1", "kind": "StatefulSet", "name": "db",
                                 "uid": "db-uid", "controller": true}],
        },
        "revision": revision,
        "data": {"spec": {"template": stored}},
    }))
}

fn statefulset_object(image: &str) -> Resource {
    object(json!({
        "apiVersion": "apps/v1", "kind": "StatefulSet",
        "metadata": {"name": "db", "namespace": "shop", "uid": "db-uid"},
        "spec": {"selector": {"matchLabels": {"app": "db"}}, "template": template("db", image, None)},
    }))
}

/// Cluster `a` connected (read-only when asked) with the fixtures in its fake store.
pub(super) struct Rollouts {
    pub h: Harness,
}

impl Rollouts {
    pub fn new(read_only: bool) -> Self {
        Self::with(read_only, deployment_object(3, "web:v3", false))
    }

    pub fn with(read_only: bool, deployment: Resource) -> Self {
        let h = Harness::with_workloads();
        h.connect("a", read_only);
        let t = Self { h };
        let port = t.port();
        port.insert(deployment);
        port.insert(replicaset("web-1", 1, "web:v1", None));
        port.insert(replicaset("web-2", 2, "web:v2", Some("bump to v2")));
        port.insert(replicaset("web-3", 3, "web:v3", Some("bump to v3")));
        port.insert(statefulset_object("db:v3"));
        for (revision, image) in [(1, "db:v1"), (2, "db:v2"), (3, "db:v3")] {
            port.insert(controller_revision(revision, image));
        }
        t
    }

    pub fn port(&self) -> Arc<FakeResourcePort> {
        self.h.resources("a")
    }

    /// Scripts the answers of the dry run and the real patch.
    pub fn answer_patches(&self) {
        let answer = deployment_object(4, "answer", false);
        self.port().script().patch.push_ok(answer.clone());
        self.port().script().patch.push_ok(answer);
    }

    /// The patches the port received, as `(dry_run, kind, body)`.
    pub fn patches(&self) -> Vec<(bool, oxikube_ports::PatchKind, Value)> {
        self.port()
            .mutating_calls()
            .into_iter()
            .map(|call| match call {
                ResourceCall::Patch { patch, options, .. } => {
                    (options.dry_run, patch.kind, patch.body)
                }
                other => panic!("unexpected write {other:?}"),
            })
            .collect()
    }
}

pub(super) fn deployment_ref() -> ResourceRef {
    ResourceRef::namespaced(id("a"), Gvk::new("apps", "v1", "Deployment"), "shop", "web")
}

pub(super) fn statefulset_ref() -> ResourceRef {
    ResourceRef::namespaced(id("a"), Gvk::new("apps", "v1", "StatefulSet"), "shop", "db")
}

pub(super) fn replicaset_ref(name: &str) -> ResourceRef {
    ResourceRef::namespaced(id("a"), Gvk::new("apps", "v1", "ReplicaSet"), "shop", name)
}

pub(super) fn rollback(target: ResourceRef, to_revision: Option<i64>) -> Command {
    Command::WorkloadRollback {
        target,
        to_revision,
    }
}
