//! Tests of the manifest -> resources mapping over the testkit fakes. Stores run on the test's
//! Tokio runtime; `settle` yields until the feeds have listed.

mod join;
mod mapping;
mod service;

use std::sync::Arc;

use futures::future::BoxFuture;
use oxikube_domain::Resource;
use oxikube_domain::helm::ReleaseRef;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk};
use oxikube_domain::kinds::{ResourceKind, VerbSet};
use oxikube_testkit::{FakeClockPort, FakeResourcePort, FakeTableFeedPort};
use serde_json::json;

use super::KindScopes;
use crate::store::{ResourceStore, StoreOptions, StorePorts, StoreRuntime};

pub(super) fn cluster() -> ClusterId {
    ClusterId::new("test", &ContextName::from("kind"))
}

pub(super) fn release() -> ReleaseRef {
    ReleaseRef::new(cluster(), "apps", "web")
}

fn kind(group: &str, version: &str, name: &str, namespaced: bool) -> ResourceKind {
    ResourceKind {
        gvk: Gvk::new(group, version, name),
        preferred: true,
        plural: format!("{}s", name.to_lowercase()),
        singular: name.to_lowercase(),
        short_names: Vec::new(),
        categories: Vec::new(),
        verbs: VerbSet::from_names(["get", "list", "watch"]),
        namespaced,
    }
}

/// Core kinds a chart uses; `Widget` (a CRD) is not served.
pub(super) fn served() -> KindScopes {
    KindScopes::from_kinds(&[
        kind("", "v1", "Pod", true),
        kind("", "v1", "ConfigMap", true),
        kind("", "v1", "ServiceAccount", true),
        kind("apps", "v1", "Deployment", true),
        kind("rbac.authorization.k8s.io", "v1", "ClusterRole", false),
    ])
}

pub(super) fn object(api_version: &str, kind: &str, ns: Option<&str>, name: &str) -> Resource {
    let mut meta = json!({"name": name, "uid": format!("uid-{name}")});
    if let Some(ns) = ns {
        meta["namespace"] = json!(ns);
    }
    Resource::from_json(json!({"apiVersion": api_version, "kind": kind, "metadata": meta}))
        .expect("resource")
}

pub(super) struct Harness {
    pub resources: Arc<FakeResourcePort>,
    pub clock: Arc<FakeClockPort>,
    pub store: ResourceStore,
}

impl Harness {
    pub fn new() -> Self {
        let clock = Arc::new(FakeClockPort::default());
        let resources = Arc::new(FakeResourcePort::with_clock(clock.clone()));
        let tables = Arc::new(FakeTableFeedPort::with_clock(clock.clone()));
        let store = ResourceStore::new(
            cluster(),
            StorePorts {
                resources: resources.clone(),
                tables,
            },
            StoreRuntime {
                spawner: Arc::new(|task: BoxFuture<'static, ()>| {
                    tokio::spawn(task);
                }),
                clock: clock.clone(),
                probe: None,
            },
            StoreOptions::default(),
        );
        Self {
            resources,
            clock,
            store,
        }
    }

    /// Lets the spawned feed tasks run.
    pub async fn settle(&self) {
        for _ in 0..64 {
            tokio::task::yield_now().await;
        }
    }
}
