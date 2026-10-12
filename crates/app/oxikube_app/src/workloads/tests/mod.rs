//! Tests of the workload commands over the real guard and the testkit fakes: [`Workloads`] is the
//! guard harness with the real handlers on it and a CronJob, a Job and a suspended Job in the
//! fake cluster `a`.

mod guard;
mod plan;
mod pods;
mod refs;
mod restart;
mod runner;
mod sanitize;
mod scale_guard;
mod scale_service;
mod set_image;
mod suspend;
mod trigger;

use oxikube_domain::Resource;
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_ports::Scale;
use oxikube_testkit::{FakeResourcePort, ResourceCall};
use serde_json::{Value, json};
use std::sync::Arc;

use crate::command_bus::{CommandOutput, DispatchContext, DispatchError};
use crate::testing::{Harness, id};

/// The harness of cluster `a` with the workload handlers.
pub(super) struct Workloads {
    pub h: Harness,
}

impl Workloads {
    /// Cluster `a` connected (read-only when asked) holding [`cronjob`] `nightly`.
    pub fn new(read_only: bool) -> Self {
        // The real `pod::Delete` handler replaces the harness's recording one.
        let h = Harness::with_workloads();
        h.connect("a", read_only);
        let t = Self { h };
        t.port().insert(cronjob("nightly"));
        t
    }

    pub fn port(&self) -> Arc<FakeResourcePort> {
        self.h.resources("a")
    }

    /// Dispatches `command` as `initiator` and confirms it, as the UI does.
    pub fn run(
        &self,
        command: Command,
        initiator: Initiator,
    ) -> Result<CommandOutput, DispatchError> {
        let ctx = DispatchContext::new(initiator, "alice");
        self.h
            .confirm_and_run(command, ctx)
            .map(|outcome| outcome.completed().expect("the handler ran"))
    }

    /// Dispatches `command` as a dry run and confirms it.
    pub fn dry_run(&self, command: Command) -> Result<CommandOutput, DispatchError> {
        let ctx = DispatchContext::new(Initiator::Agent, "agent").with_dry_run(true);
        self.h
            .confirm_and_run(command, ctx)
            .map(|outcome| outcome.completed().expect("the handler ran"))
    }

    /// The objects of kind `gvk` in the fake cluster.
    pub fn objects_of(&self, gvk: &Gvk) -> Vec<Resource> {
        self.port()
            .objects()
            .into_iter()
            .filter(|object| &object.kind == gvk)
            .collect()
    }

    /// The writes the port received, as `(kind, method, dry_run)`.
    pub fn writes(&self) -> Vec<(String, &'static str, bool)> {
        self.port()
            .mutating_calls()
            .iter()
            .map(|call| {
                let (kind, method) = match call {
                    ResourceCall::Create { kind, .. } => (kind, "create"),
                    ResourceCall::Patch { kind, .. } => (kind, "patch"),
                    other => panic!("unexpected write {other:?}"),
                };
                (kind.kind.to_string(), method, call.is_dry_run())
            })
            .collect()
    }

    /// The body of write `n`.
    pub fn body(&self, n: usize) -> Value {
        match &self.port().mutating_calls()[n] {
            ResourceCall::Create { object, .. } => object.clone(),
            ResourceCall::Patch { patch, .. } => patch.body.clone(),
            other => panic!("not a write with a body: {other:?}"),
        }
    }
}

/// A CronJob `name` in `default` with a job template.
pub(super) fn cronjob(name: &str) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "batch/v1", "kind": "CronJob",
        "metadata": {"name": name, "namespace": "default", "uid": "cj-uid"},
        "spec": {
            "schedule": "0 3 * * *",
            "concurrencyPolicy": "Forbid",
            "suspend": false,
            "jobTemplate": {
                "metadata": {"labels": {"team": "data"}, "annotations": {"note": "x"}},
                "spec": {"backoffLimit": 2, "template": {"spec": {
                    "restartPolicy": "Never",
                    "containers": [{"name": "run", "image": "busybox:1"}],
                }}},
            },
        },
    }))
    .expect("a valid object")
}

/// A Job `name` in `default` with `status`.
pub(super) fn job(name: &str, spec: Value, status: Value) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "batch/v1", "kind": "Job",
        "metadata": {"name": name, "namespace": "default", "uid": format!("{name}-uid")},
        "spec": spec, "status": status,
    }))
    .expect("a valid object")
}

pub(super) fn cronjob_ref(name: &str) -> ResourceRef {
    ResourceRef::namespaced(id("a"), Gvk::new("batch", "v1", "CronJob"), "default", name)
}

pub(super) fn job_ref(name: &str) -> ResourceRef {
    ResourceRef::namespaced(id("a"), Gvk::new("batch", "v1", "Job"), "default", name)
}

pub(super) fn trigger(name: &str) -> Command {
    Command::CronJobTrigger {
        target: cronjob_ref(name),
    }
}

/// A scalable workload `name` of `kind` in the `shop` namespace of `cluster`.
pub(super) fn workload(cluster: &str, group: &str, kind: &str, name: &str) -> ResourceRef {
    ResourceRef::namespaced(id(cluster), Gvk::new(group, "v1", kind), "shop", name)
}

/// Deployment `shop/web` of `cluster`.
pub(super) fn deployment(cluster: &str) -> ResourceRef {
    workload(cluster, "apps", "Deployment", "web")
}

pub(super) fn scale_to(target: ResourceRef, replicas: u32) -> Command {
    Command::WorkloadScale { target, replicas }
}

/// The `scale` subresource as the server answers it.
pub(super) fn scale(spec: i32, status: i32) -> Scale {
    Scale {
        replicas: spec,
        current_replicas: status,
        selector: None,
        resource_version: None,
    }
}

/// A Pod `name` in `default` in `phase`, owned by the ReplicaSet `web-5d` (a controller).
pub(super) fn pod(name: &str, phase: &str) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": {
            "name": name, "namespace": "default", "uid": format!("{name}-uid"),
            "ownerReferences": [{
                "apiVersion": "apps/v1", "kind": "ReplicaSet", "name": "web-5d",
                "uid": "rs-uid", "controller": true
            }],
        },
        "spec": {"nodeName": "worker-1", "containers": [{"name": "app", "image": "web:1"}]},
        "status": {"phase": phase},
    }))
    .expect("a valid object")
}

/// A Deployment, StatefulSet or DaemonSet `name` of `kind` in `shop` with an init container
/// `migrate`, a container `web` and a container `sidecar`, `annotations` on its pod template
/// (none when `None`: the map is absent, as in a hand-written manifest) and resourceVersion `7`.
pub(super) fn rollable(kind: &str, name: &str, annotations: Option<Value>) -> Resource {
    let mut template_meta = json!({"labels": {"app": name}});
    if let Some(annotations) = annotations {
        template_meta["annotations"] = annotations;
    }
    Resource::from_json(json!({
        "apiVersion": "apps/v1", "kind": kind,
        "metadata": {
            "name": name, "namespace": "shop", "uid": format!("{name}-uid"),
            "resourceVersion": "7",
        },
        "spec": {"template": {"metadata": template_meta, "spec": {
            "initContainers": [{"name": "migrate", "image": "app:1", "args": ["up"]}],
            "containers": [
                {"name": "web", "image": "nginx:1.26", "ports": [{"containerPort": 80}]},
                {"name": "sidecar", "image": "envoy:1"},
            ],
        }}},
    }))
    .expect("a valid object")
}

pub(super) fn pod_ref(name: &str) -> ResourceRef {
    ResourceRef::namespaced(id("a"), Gvk::new("", "v1", "Pod"), "default", name)
}

pub(super) fn restart_of(target: ResourceRef) -> Command {
    Command::WorkloadRestart { target }
}

pub(super) fn set_image_of(target: ResourceRef, images: &[(&str, &str)]) -> Command {
    Command::WorkloadSetImage {
        target,
        images: images
            .iter()
            .map(|(container, image)| oxikube_domain::command::ImageChange::new(*container, *image))
            .collect(),
    }
}
