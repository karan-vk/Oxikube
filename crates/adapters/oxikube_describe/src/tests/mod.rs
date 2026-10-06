//! Tests: the native backend on a recorded object, the `kubectl` backend against a stub script,
//! and the choice between them.

mod describer;
mod kubectl;
mod native;

use std::sync::Arc;

use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_domain::kinds::{ResourceKind, VerbSet};
use oxikube_testkit::FakeDiscoveryPort;

pub(crate) fn kind(group: &str, version: &str, name: &str, plural: &str) -> ResourceKind {
    ResourceKind {
        gvk: Gvk::new(group, version, name),
        preferred: true,
        plural: plural.into(),
        singular: name.to_lowercase(),
        short_names: Vec::new(),
        categories: Vec::new(),
        verbs: VerbSet::from_names(["get", "list", "watch"]),
        namespaced: true,
    }
}

pub(crate) fn discovery() -> Arc<FakeDiscoveryPort> {
    Arc::new(FakeDiscoveryPort::new().with_kinds([
        kind("", "v1", "Pod", "pods"),
        kind("apps", "v1", "Deployment", "deployments"),
        kind("test.oxikube.dev", "v1", "Widget", "widgets"),
    ]))
}

pub(crate) fn cluster() -> ClusterId {
    ClusterId::new("test", &ContextName::new("kind-test"))
}

pub(crate) fn pod_ref() -> ResourceRef {
    ResourceRef::namespaced(cluster(), Gvk::new("", "v1", "Pod"), "demo", "web-running")
}
