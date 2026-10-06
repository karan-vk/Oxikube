//! Catalog service and command tests against `oxikube_testkit` fakes. No runtime, no threads:
//! the fakes complete at once and futures are polled with `now_or_never`.

mod commands;
mod entries;
mod marks;

use std::sync::Arc;

use futures::FutureExt as _;
use jiff::Timestamp;
use oxikube_domain::OxiResult;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_ports::{ClusterContext, ClusterSource, SourceId, SourceKind};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeStatePort,
};

use super::{CatalogEntry, ClusterCatalog};
use crate::session::ClusterSessionManager;

/// The source every test context comes from.
pub(super) const SOURCE: &str = "default";

/// A context `name` of the default source, with a cluster and user named after it.
pub(super) fn ctx(name: &str) -> ClusterContext {
    let context = ContextName::new(name);
    ClusterContext {
        server: Some(format!("https://{name}.example:6443")),
        cluster_name: Some(format!("{name}-cluster")),
        user: Some(format!("{name}-user")),
        ..ClusterContext::new(
            ClusterId::new("/home/me/.kube/config", &context),
            context,
            SourceId(SOURCE.into()),
        )
    }
}

pub(super) fn id(name: &str) -> ClusterId {
    ctx(name).cluster
}

/// The default source: one kubeconfig file.
pub(super) fn source() -> ClusterSource {
    ClusterSource {
        id: SourceId(SOURCE.into()),
        kind: SourceKind::KubeconfigFile,
        label: "~/.kube/config".into(),
        path: Some("/home/me/.kube/config".into()),
    }
}

/// The catalog under test with its fakes. The source holds contexts `a`, `b` and `c`.
pub(super) struct Harness {
    pub(super) catalog: ClusterCatalog,
    pub(super) sessions: ClusterSessionManager,
    pub(super) source: Arc<FakeClusterSourcePort>,
    pub(super) state: Arc<FakeStatePort>,
    pub(super) clock: Arc<FakeClockPort>,
    pub(super) connector: Arc<FakeClusterConnectorPort>,
}

impl Harness {
    pub(super) fn new() -> Self {
        let source = Arc::new(
            FakeClusterSourcePort::new()
                .with_sources([source()])
                .with_contexts([ctx("a"), ctx("b"), ctx("c")]),
        );
        let state = Arc::new(FakeStatePort::new());
        let clock = Arc::new(FakeClockPort::default());
        let connector = Arc::new(FakeClusterConnectorPort::new());
        let catalog = ClusterCatalog::new(source.clone(), state.clone(), clock.clone());
        let sessions = ClusterSessionManager::new(connector.clone(), source.clone(), clock.clone());
        Self {
            catalog,
            sessions,
            source,
            state,
            clock,
            connector,
        }
    }

    pub(super) fn load(&self) -> Vec<CatalogEntry> {
        self.try_load().expect("load")
    }

    pub(super) fn try_load(&self) -> OxiResult<Vec<CatalogEntry>> {
        self.catalog
            .load()
            .now_or_never()
            .expect("load should not wait")
    }

    pub(super) fn entry(&self, name: &str) -> CatalogEntry {
        self.load()
            .into_iter()
            .find(|e| e.name() == name)
            .unwrap_or_else(|| panic!("{name} is in the catalog"))
    }

    pub(super) fn now(&self) -> Timestamp {
        use oxikube_ports::ClockPort as _;
        self.clock.now()
    }
}
