//! Namespace service tests against `oxikube_testkit` fakes: no runtime, no threads. The fakes
//! answer at once, and the debounce waits on the virtual clock.

mod catalog;
mod command;
mod debounce;
mod now;
mod persist;
mod select;
mod shortcuts;

use std::sync::Arc;

use futures::{FutureExt, StreamExt};
use oxikube_domain::Resource;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_ports::{ClusterContext, SourceId};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeStatePort,
};
use serde_json::json;

use super::NamespaceService;
use crate::session::{ClusterSessionManager, SessionChange, SessionUpdate, SessionUpdates};

fn ctx(name: &str) -> ClusterContext {
    let context = ContextName::new(name);
    let cluster = ClusterId::new("/home/me/.kube/config", &context);
    ClusterContext {
        server: Some(format!("https://{name}.example:6443")),
        ..ClusterContext::new(cluster, context, SourceId("kubeconfig".into()))
    }
}

fn id(name: &str) -> ClusterId {
    ctx(name).cluster
}

fn namespace(name: &str) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1",
        "kind": "Namespace",
        "metadata": { "name": name },
    }))
    .expect("namespace json")
}

/// A manager with clusters `a` and `b` in its catalog, both open, a service over an in-memory
/// `StatePort`, and an update subscription taken before anything happened.
struct Harness {
    manager: ClusterSessionManager,
    connector: Arc<FakeClusterConnectorPort>,
    state: Arc<FakeStatePort>,
    clock: Arc<FakeClockPort>,
    service: NamespaceService,
    updates: SessionUpdates,
}

impl Harness {
    fn new() -> Self {
        let connector = Arc::new(FakeClusterConnectorPort::new());
        let source = Arc::new(FakeClusterSourcePort::new().with_contexts([ctx("a"), ctx("b")]));
        let clock = Arc::new(FakeClockPort::default());
        let manager = ClusterSessionManager::new(connector.clone(), source, clock.clone());
        let state = Arc::new(FakeStatePort::new());
        let service = NamespaceService::new(manager.clone(), state.clone(), clock.clone());
        for name in ["a", "b"] {
            manager.open(&ctx(name), Default::default());
        }
        let updates = manager.subscribe();
        Self {
            manager,
            connector,
            state,
            clock,
            service,
            updates,
        }
    }

    /// Lists these namespaces on cluster `name` and connects it.
    fn connect(&self, name: &str, namespaces: &[&str]) {
        let ports = self.connector.ports_for(&id(name));
        for ns in namespaces {
            ports.resources.insert(namespace(ns));
        }
        self.run(self.manager.connect(&id(name))).expect("connect");
    }

    /// A second service over the same manager, state and clock: what a restart looks like to
    /// the stored data.
    fn restarted(&self) -> NamespaceService {
        NamespaceService::new(self.manager.clone(), self.state.clone(), self.clock.clone())
    }

    fn run<T>(&self, fut: impl std::future::Future<Output = T>) -> T {
        futures::executor::block_on(fut)
    }

    fn selection(&self, name: &str) -> oxikube_domain::session::NamespaceSelection {
        self.manager
            .get(&id(name))
            .expect("session")
            .namespace_selection()
            .clone()
    }

    /// Every `NamespaceChanged` sent since the last drain, as `(cluster, selection)`.
    fn namespace_changes(
        &mut self,
    ) -> Vec<(ClusterId, oxikube_domain::session::NamespaceSelection)> {
        let mut out = Vec::new();
        while let Some(Some(item)) = self.updates.next().now_or_never() {
            let SessionUpdate { cluster, change } = item.expect("subscriber lagged");
            if let SessionChange::NamespaceChanged(selection) = change {
                out.push((cluster, selection));
            }
        }
        out
    }
}
