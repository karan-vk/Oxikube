//! Tests of the bulk commands over the real guard and the testkit fakes: [`Selected`] is the
//! guard harness with the real `selection::*` handlers on it and a small cluster `a`.

mod concurrency;
mod flow;
mod guard;
mod label;
mod restart;
mod run;

use std::sync::Arc;

use oxikube_domain::Resource;
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::{Command, Propagation};
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_testkit::{FakeResourcePort, ResourceCall};
use serde_json::{Value, json};

use crate::command_bus::{CommandOutput, DispatchContext, DispatchError};
use crate::guard::Confirmation;
use crate::selection::SelectionFlow;
use crate::testing::{Harness, id};

/// The harness of cluster `a` with the bulk handlers.
pub(super) struct Selected {
    pub h: Harness,
}

impl Selected {
    /// Cluster `a` connected (read-only when asked).
    pub fn new(read_only: bool) -> Self {
        let h = Harness::with_selection();
        h.connect("a", read_only);
        Self { h }
    }

    pub fn port(&self) -> Arc<FakeResourcePort> {
        self.h.resources("a")
    }

    pub fn flow(&self) -> SelectionFlow {
        SelectionFlow::new(self.h.bus.clone(), self.h.manager.clone(), "alice")
    }

    /// Dispatches `command` as `initiator` and confirms it, as the UI does.
    pub fn run(
        &self,
        command: Command,
        initiator: Initiator,
    ) -> Result<CommandOutput, DispatchError> {
        self.h
            .confirm_and_run(command, DispatchContext::new(initiator, "alice"))
            .map(|outcome| outcome.completed().expect("the handler ran"))
    }

    /// Like [`run`](Self::run), but the port's writes yield while in flight (see
    /// `FakeResourcePort::track_write_overlap`), so the batch runs on an executor that re-polls and
    /// the port can tell how many writes overlapped.
    pub fn run_overlapping(
        &self,
        command: Command,
        initiator: Initiator,
    ) -> Result<CommandOutput, DispatchError> {
        let ctx = DispatchContext::new(initiator, "alice");
        let request = self.h.ask(command.clone(), ctx.clone());
        let answer = match request.expected_name {
            Some(name) => Confirmation::typed(request.token, name),
            None => Confirmation::simple(request.token),
        };
        self.port().track_write_overlap();
        futures::executor::block_on(self.h.bus.dispatch(command, ctx.with_confirmation(answer)))
            .map(|outcome| outcome.completed().expect("the handler ran"))
    }

    /// The writes the port received, as `(method, name, dry_run)`.
    pub fn writes(&self) -> Vec<(&'static str, String, bool)> {
        self.port()
            .mutating_calls()
            .iter()
            .map(|call| {
                let (method, name) = match call {
                    ResourceCall::Delete { name, .. } => ("delete", name.clone()),
                    ResourceCall::Patch { name, .. } => ("patch", name.clone()),
                    other => panic!("unexpected write {other:?}"),
                };
                (method, name, call.is_dry_run())
            })
            .collect()
    }

    /// The object `name` of `kind` in the fake cluster.
    pub fn object(&self, kind: &str, name: &str) -> Option<Resource> {
        self.port()
            .objects()
            .into_iter()
            .find(|o| &*o.kind.kind == kind && o.name() == name)
    }
}

pub(super) fn ns_ref(kind: &str, group: &str, namespace: &str, name: &str) -> ResourceRef {
    ResourceRef::namespaced(id("a"), Gvk::new(group, "v1", kind), namespace, name)
}

pub(super) fn pod_ref(name: &str) -> ResourceRef {
    ns_ref("Pod", "", "default", name)
}

pub(super) fn deployment_ref(name: &str) -> ResourceRef {
    ns_ref("Deployment", "apps", "shop", name)
}

pub(super) fn namespace_ref(name: &str) -> ResourceRef {
    ResourceRef::cluster_scoped(id("a"), Gvk::new("", "v1", "Namespace"), name)
}

pub(super) fn pod(name: &str, namespace: &str, labels: Value) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": {"name": name, "namespace": namespace, "uid": format!("{name}-uid"),
                     "labels": labels},
        "spec": {"containers": [{"name": "app", "image": "web:1"}]},
        "status": {"phase": "Running"},
    }))
    .expect("a valid object")
}

pub(super) fn deployment(name: &str) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "apps/v1", "kind": "Deployment",
        "metadata": {"name": name, "namespace": "shop", "uid": format!("{name}-uid"),
                     "labels": {"app": name}},
        "spec": {"replicas": 2, "template": {"metadata": {"labels": {"app": name}},
                 "spec": {"containers": [{"name": "web", "image": "web:1"}]}}},
    }))
    .expect("a valid object")
}

pub(super) fn namespace(name: &str) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1", "kind": "Namespace",
        "metadata": {"name": name, "uid": format!("{name}-uid")},
    }))
    .expect("a valid object")
}

pub(super) fn delete_of(targets: Vec<ResourceRef>) -> Command {
    Command::SelectionDelete {
        targets,
        propagation: Propagation::Background,
    }
}

pub(super) fn restart_of(targets: Vec<ResourceRef>) -> Command {
    Command::SelectionRestart { targets }
}
