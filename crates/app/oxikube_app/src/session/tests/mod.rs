//! Session manager tests against `oxikube_testkit` fakes. No runtime, no threads: the
//! fakes complete at once (or wait on `hold` / the virtual clock) and futures are polled
//! by hand with `now_or_never`.

mod auth;
mod cancel;
mod connect;
mod deadline;
mod failure;
mod health;
mod multi;
mod prefs;
mod props;

use std::sync::Arc;

use futures::{FutureExt, StreamExt};
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_domain::session::{ClusterSessionState, SessionPhase};
use oxikube_ports::{ClusterContext, SourceId};
use oxikube_testkit::{FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort};

use super::{
    ClusterSessionManager, SessionChange, SessionManagerConfig, SessionUpdate, SessionUpdates,
};

/// A catalog entry for context `name` from one kubeconfig file.
fn ctx(name: &str) -> ClusterContext {
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

fn id(name: &str) -> ClusterId {
    ctx(name).cluster
}

/// The manager under test with its fakes and an update subscription taken before
/// anything happened. The catalog holds contexts `a` and `b`.
struct Harness {
    manager: ClusterSessionManager,
    connector: Arc<FakeClusterConnectorPort>,
    source: Arc<FakeClusterSourcePort>,
    clock: Arc<FakeClockPort>,
    updates: SessionUpdates,
}

impl Harness {
    fn new() -> Self {
        Self::with_config(SessionManagerConfig::default())
    }

    fn with_config(config: SessionManagerConfig) -> Self {
        let connector = Arc::new(FakeClusterConnectorPort::new());
        let source = Arc::new(FakeClusterSourcePort::new().with_contexts([ctx("a"), ctx("b")]));
        let clock = Arc::new(FakeClockPort::default());
        let manager = ClusterSessionManager::with_config(
            connector.clone(),
            source.clone(),
            clock.clone(),
            config,
        );
        let updates = manager.subscribe();
        Self {
            manager,
            connector,
            source,
            clock,
            updates,
        }
    }

    /// Runs `connect` to completion; panics when it would wait.
    fn connect(&self, name: &str) -> ClusterSessionState {
        self.manager
            .connect(&id(name))
            .now_or_never()
            .expect("connect should not wait")
            .expect("connect")
    }

    /// Runs `reconnect` to completion; panics when it would wait.
    fn reconnect(&self, name: &str) -> ClusterSessionState {
        self.manager
            .reconnect(&id(name))
            .now_or_never()
            .expect("reconnect should not wait")
            .expect("reconnect")
    }

    fn phase(&self, name: &str) -> SessionPhase {
        self.manager.get(&id(name)).expect("session").phase()
    }

    /// Every update sent since the last drain.
    fn drain(&mut self) -> Vec<SessionUpdate> {
        let mut out = Vec::new();
        while let Some(Some(item)) = self.updates.next().now_or_never() {
            out.push(item.expect("subscriber lagged"));
        }
        out
    }

    /// The phases `name` moved through since the last drain (other updates are dropped).
    fn phases(&mut self, name: &str) -> Vec<SessionPhase> {
        let cluster = id(name);
        self.drain()
            .into_iter()
            .filter(|u| u.cluster == cluster)
            .filter_map(|u| match u.change {
                SessionChange::StateChanged { state, .. } => Some(state.phase()),
                _ => None,
            })
            .collect()
    }
}
