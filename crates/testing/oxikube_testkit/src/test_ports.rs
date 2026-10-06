//! [`TestPorts`]: one bundle of port fakes, built the same way in every test.
//!
//! The binary's `AppState` holds the ports of the running app (`Arc<dyn Port>`). A test needs the
//! same shape, but also needs to keep the concrete fakes to script them and to assert on their
//! recorded calls. `TestPorts` holds both views: the fields are the typed fakes, and each is
//! cheap to clone (`Arc`) into whatever expects a trait object.
//!
//! ```
//! use std::sync::Arc;
//! use oxikube_ports::StatePort;
//! use oxikube_testkit::TestPorts;
//!
//! let ports = TestPorts::seeded();
//! let state: Arc<dyn StatePort> = ports.state.clone();
//! // ... hand `state` to the code under test, then assert on `ports.state.recorded_calls()`.
//! # let _ = state;
//! ```
//!
//! `AppState::test` / `AppState::test_with` in `bins/oxikube` (feature `test-support`) build the
//! real `AppState` from these fakes, so a `#[gpui::test]` gets the production init order over
//! in-memory ports.

use std::sync::Arc;

use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_ports::{ClusterContext, ClusterSource, SourceId, SourceKind};

use crate::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeFsPort, FakeResourcePort,
    FakeSecretStorePort, FakeStatePort, fixtures,
};

/// The fakes that make up a test app. See the [module docs](self).
#[derive(Clone)]
pub struct TestPorts {
    /// In-memory durable state (`StatePort`): kv store, typed tables, audit log.
    pub state: Arc<FakeStatePort>,
    /// In-memory keychain (`SecretStorePort`).
    pub secrets: Arc<FakeSecretStorePort>,
    /// The cluster catalog (`ClusterSourcePort`).
    pub clusters: Arc<FakeClusterSourcePort>,
    /// Connects the catalog's contexts (`ClusterConnectorPort`): every connect succeeds with a
    /// bundle of fakes unless the test scripts it.
    pub connector: Arc<FakeClusterConnectorPort>,
    /// Local files by path (`FsPort`), in memory.
    pub fs: Arc<FakeFsPort>,
    /// The cluster's objects (`ResourcePort`), replayed on [`TestPorts::clock`].
    pub resources: Arc<FakeResourcePort>,
    /// The virtual clock the fakes replay streams on. It is independent of GPUI's test clock:
    /// advance both (`FakeClockPort::advance` and `TestApp::advance_clock`) when a test needs both.
    pub clock: Arc<FakeClockPort>,
}

impl TestPorts {
    /// The context name of the seeded cluster.
    pub const CONTEXT: &'static str = "kind-oxikube";
    /// The id of the source the seeded context comes from.
    pub const SOURCE: &'static str = "test-kubeconfig";

    /// Fakes with no data: no clusters, no objects, empty state.
    pub fn empty() -> Self {
        let clock = Arc::new(FakeClockPort::default());
        Self {
            state: Arc::new(FakeStatePort::new()),
            secrets: Arc::new(FakeSecretStorePort::new()),
            clusters: Arc::new(FakeClusterSourcePort::new()),
            connector: Arc::new(FakeClusterConnectorPort::new()),
            fs: Arc::new(FakeFsPort::new()),
            resources: Arc::new(FakeResourcePort::with_clock(clock.clone())),
            clock,
        }
    }

    /// Fakes with one kubeconfig source holding one context ([`TestPorts::CONTEXT`], see
    /// [`TestPorts::cluster_id`]), and a small cluster: a running and a crash-looping pod, a
    /// Deployment, a node and a namespace (the fixtures of `oxikube_testkit::fixtures`).
    pub fn seeded() -> Self {
        let ports = Self::empty();
        let source = SourceId(Self::SOURCE.to_owned());
        let clusters = FakeClusterSourcePort::new()
            .with_sources([ClusterSource {
                id: source.clone(),
                kind: SourceKind::KubeconfigFile,
                label: "Test kubeconfig".to_owned(),
                path: None,
            }])
            .with_contexts([ClusterContext {
                server: Some("https://127.0.0.1:6443".to_owned()),
                default_namespace: Some("default".to_owned()),
                cluster_name: Some(Self::CONTEXT.to_owned()),
                user: Some(Self::CONTEXT.to_owned()),
                ..ClusterContext::new(Self::cluster_id(), ContextName::new(Self::CONTEXT), source)
            }]);
        for object in [
            fixtures::pod_running(),
            fixtures::pod_crashloop(),
            fixtures::deployment_ready(),
            fixtures::node_ready(),
            fixtures::namespace(),
        ] {
            ports.resources.insert(object);
        }
        Self {
            clusters: Arc::new(clusters),
            ..ports
        }
    }

    /// The [`ClusterId`] of the seeded context.
    pub fn cluster_id() -> ClusterId {
        ClusterId::new(Self::SOURCE, &ContextName::new(Self::CONTEXT))
    }
}

impl Default for TestPorts {
    fn default() -> Self {
        Self::empty()
    }
}

#[cfg(test)]
mod tests {
    use oxikube_ports::ClusterSourcePort as _;

    use super::*;

    #[test]
    fn the_seeded_ports_hold_one_cluster_and_some_objects() {
        let ports = TestPorts::seeded();
        let contexts = futures::executor::block_on(ports.clusters.contexts()).expect("contexts");
        assert_eq!(contexts.len(), 1);
        assert_eq!(contexts[0].cluster, TestPorts::cluster_id());
        assert_eq!(ports.resources.objects().len(), 5);
    }

    #[test]
    fn the_empty_ports_hold_nothing() {
        let ports = TestPorts::empty();
        let contexts = futures::executor::block_on(ports.clusters.contexts()).expect("contexts");
        assert!(contexts.is_empty());
    }
}
