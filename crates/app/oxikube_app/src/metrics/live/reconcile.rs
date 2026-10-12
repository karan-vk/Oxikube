//! What keeps the pollers in step with the world: [`Inner::reconcile`] starts or stops a
//! cluster's poller from who watches it, and [`follow`] reacts to the session updates.

use std::sync::{Arc, Weak};

use futures::StreamExt as _;
use oxikube_domain::ids::ClusterId;

use super::cadence::Cadence;
use super::poller::poll_loop;
use super::service::Inner;
use super::state::MetricsKind;
use crate::session::{SessionChange, SessionUpdates};
use crate::store::spawn_guarded;

impl Inner {
    pub(super) fn notify(&self, cluster: &ClusterId) {
        let _ = self.changes.send(cluster.clone());
    }

    pub(super) fn reconcile_all(self: &Arc<Self>) {
        let clusters: Vec<ClusterId> = self.state.lock().clusters.keys().cloned().collect();
        for cluster in clusters {
            self.reconcile(&cluster);
        }
    }

    /// Starts `cluster`'s poller when it is watched, shown and connected, and stops it when it is
    /// not.
    pub(super) fn reconcile(self: &Arc<Self>, cluster: &ClusterId) {
        let connected = self
            .sessions
            .get(cluster)
            .is_some_and(|session| session.is_connected());
        let mut state = self.state.lock();
        let spawner = state.spawner.clone();
        let Some(entry) = state.clusters.get_mut(cluster) else {
            return;
        };
        let watched = entry.visible > 0 && entry.wanted().next().is_some() && connected;
        match (watched, entry.poller.is_some(), spawner) {
            (true, false, Some(spawner)) => {
                for cadence in &mut entry.cadence {
                    cadence.resume();
                }
                let task = poll_loop(Arc::downgrade(self), cluster.clone());
                entry.poller = Some(spawn_guarded(&spawner, task));
            }
            (false, true, _) => entry.poller = None,
            _ => {}
        }
    }

    /// The cluster dropped its connection (or closed): stop, forget its samples and history.
    fn disconnected(self: &Arc<Self>, cluster: &ClusterId) {
        let usage = {
            let mut state = self.state.lock();
            let Some(entry) = state.clusters.get_mut(cluster) else {
                return;
            };
            entry.poller = None;
            entry.cadence = Default::default();
            entry.usage.clone()
        };
        usage.clear();
        self.notify(cluster);
    }

    /// The namespace selection moved: the pods are read again, now.
    fn namespaces_changed(self: &Arc<Self>, cluster: &ClusterId) {
        let usage = {
            let mut state = self.state.lock();
            let Some(entry) = state.clusters.get_mut(cluster) else {
                return;
            };
            entry.cadence[MetricsKind::Pods.index()] = Cadence::default();
            entry.poller = None;
            entry.usage.clone()
        };
        usage.clear_pods();
        self.notify(cluster);
        self.reconcile(cluster);
    }
}

/// Reacts to the session updates until the service is dropped.
pub(super) async fn follow(inner: Weak<Inner>, mut updates: SessionUpdates) {
    while let Some(item) = updates.next().await {
        let Some(inner) = inner.upgrade() else {
            return;
        };
        match item {
            Ok(update) => match update.change {
                SessionChange::StateChanged { from, state } => {
                    let connected = state.phase().is_connected();
                    if !connected && from.is_connected() {
                        inner.disconnected(&update.cluster);
                    } else if connected && !from.is_connected() {
                        inner.reconcile(&update.cluster);
                    }
                }
                SessionChange::Closed => inner.disconnected(&update.cluster),
                SessionChange::NamespaceChanged(_) => inner.namespaces_changed(&update.cluster),
                _ => {}
            },
            Err(_lagged) => inner.reconcile_all(),
        }
    }
}
