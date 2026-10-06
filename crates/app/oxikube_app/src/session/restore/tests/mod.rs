//! Session restore tests against `oxikube_testkit` fakes: no runtime, no threads. The fakes answer
//! at once, or wait on `hold` and the virtual clock, and futures are polled by hand.

mod connect;
mod failures;
mod plan;
mod prepare;

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use futures::executor::block_on;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_domain::session::{ClusterSessionState, SessionPhase};
use oxikube_ports::{ClusterContext, SourceId};
use oxikube_testkit::{
    ConnectorCall, FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeStatePort,
};

use super::{ClusterTabsStore, RestoreConfig, SavedTabs, SessionRestorer};
use crate::session::ClusterSessionManager;
use crate::session::namespaces::NamespaceService;

pub(super) fn ctx(name: &str) -> ClusterContext {
    let context = ContextName::new(name);
    ClusterContext {
        server: Some(format!("https://{name}.example:6443")),
        ..ClusterContext::new(
            ClusterId::new("/home/me/.kube/config", &context),
            context,
            SourceId("kubeconfig".into()),
        )
    }
}

pub(super) fn id(name: &str) -> ClusterId {
    ctx(name).cluster
}

/// The restorer under test with its fakes. The catalog holds contexts `a`, `b` and `c`; the saved
/// session is whatever the test saves.
pub(super) struct Harness {
    pub(super) sessions: ClusterSessionManager,
    pub(super) connector: Arc<FakeClusterConnectorPort>,
    pub(super) source: Arc<FakeClusterSourcePort>,
    pub(super) clock: Arc<FakeClockPort>,
    pub(super) state: Arc<FakeStatePort>,
    pub(super) store: ClusterTabsStore,
    pub(super) namespaces: NamespaceService,
    pub(super) restorer: SessionRestorer,
}

impl Harness {
    pub(super) fn new() -> Self {
        Self::with_config(RestoreConfig::default())
    }

    pub(super) fn with_config(config: RestoreConfig) -> Self {
        Self::on_state(Arc::new(FakeStatePort::new()), config)
    }

    /// A new launch over the state a previous one left.
    pub(super) fn on_state(state: Arc<FakeStatePort>, config: RestoreConfig) -> Self {
        let connector = Arc::new(FakeClusterConnectorPort::new());
        let source =
            Arc::new(FakeClusterSourcePort::new().with_contexts([ctx("a"), ctx("b"), ctx("c")]));
        let clock = Arc::new(FakeClockPort::default());
        let sessions = ClusterSessionManager::new(connector.clone(), source.clone(), clock.clone());
        let store = ClusterTabsStore::new(state.clone(), "main").expect("store");
        let namespaces = NamespaceService::new(sessions.clone(), state.clone(), clock.clone());
        let restorer = SessionRestorer::new(
            sessions.clone(),
            namespaces.clone(),
            source.clone(),
            store.clone(),
            config,
        );
        Self {
            sessions,
            connector,
            source,
            clock,
            state,
            store,
            namespaces,
            restorer,
        }
    }

    /// Saves `open` (context names) with `active` displayed, as the cluster tabs do.
    pub(super) fn save(&self, open: &[&str], active: Option<&str>) {
        let saved = SavedTabs::new(open.iter().map(|n| id(n)).collect(), active.map(id));
        block_on(self.store.save(&saved)).expect("save");
    }

    pub(super) fn saved(&self) -> Option<SavedTabs> {
        block_on(self.store.load()).expect("load")
    }

    pub(super) fn state_of(&self, name: &str) -> ClusterSessionState {
        self.sessions
            .get(&id(name))
            .expect("session")
            .state()
            .clone()
    }

    pub(super) fn phase(&self, name: &str) -> SessionPhase {
        self.state_of(name).phase()
    }

    /// The clusters the connector was asked to connect, in call order, as context names.
    pub(super) fn connects(&self) -> Vec<String> {
        self.connector
            .recorded_calls()
            .into_iter()
            .map(|ConnectorCall::Connect { context, .. }| context.to_string())
            .collect()
    }

    /// The sessions that exist, in open order, as context names.
    pub(super) fn open_names(&self) -> Vec<String> {
        self.sessions
            .sessions()
            .iter()
            .map(|s| s.context().to_string())
            .collect()
    }
}

/// Polls `fut` once and reports whether it finished (it must not when it waits on a hold or on
/// the virtual clock).
pub(super) fn poll_once<F: Future + Unpin>(fut: &mut F) -> bool {
    futures::executor::block_on(async { futures::poll!(fut).is_ready() })
}

pub(super) const TIMEOUT: Duration = Duration::from_secs(30);
