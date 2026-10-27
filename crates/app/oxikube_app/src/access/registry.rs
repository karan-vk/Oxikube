//! [`AccessRegistry`]: one [`AccessService`] per connected cluster.

use std::collections::HashMap;
use std::sync::{Arc, Weak};
use std::time::Duration;

use futures::FutureExt as _;
use futures::StreamExt as _;
use futures::future::BoxFuture;
use oxikube_domain::ids::ClusterId;
use oxikube_ports::{AccessReviewPort, ClockPort};
use parking_lot::Mutex;

use super::service::{AccessService, DEFAULT_TTL};
use crate::session::{ClusterSession, ClusterSessionManager, SessionChange};

type Slots = Mutex<HashMap<ClusterId, Slot>>;

struct Slot {
    port: Arc<dyn AccessReviewPort>,
    service: AccessService,
}

/// Hands out the [`AccessService`] of each connected cluster and starts a fresh one when the
/// cluster reconnects (a new connection has a new port), which is the "invalidate on connect and
/// context switch" rule. Cheap to clone; clones share the services.
///
/// A slot holds the session's port (its kube client pool and discovery), so a cluster that is
/// closed or disconnected must be dropped from here too: the host runs [`follow`](Self::follow)
/// for that.
#[derive(Clone)]
pub struct AccessRegistry {
    clock: Arc<dyn ClockPort>,
    ttl: Duration,
    slots: Arc<Slots>,
}

impl std::fmt::Debug for AccessRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccessRegistry")
            .field("clusters", &self.slots.lock().len())
            .finish_non_exhaustive()
    }
}

impl AccessRegistry {
    /// A registry whose services cache for [`DEFAULT_TTL`].
    pub fn new(clock: Arc<dyn ClockPort>) -> Self {
        Self::with_ttl(clock, DEFAULT_TTL)
    }

    /// A registry whose services cache for `ttl`.
    pub fn with_ttl(clock: Arc<dyn ClockPort>, ttl: Duration) -> Self {
        Self {
            clock,
            ttl,
            slots: Arc::default(),
        }
    }

    /// The service of `cluster`'s current connection; `None` while it is not connected.
    pub fn for_session(
        &self,
        cluster: &ClusterId,
        session: &ClusterSession,
    ) -> Option<AccessService> {
        let port = session.access()?;
        let mut slots = self.slots.lock();
        if let Some(slot) = slots.get(cluster)
            && Arc::ptr_eq(&slot.port, &port)
        {
            return Some(slot.service.clone());
        }
        let service = AccessService::new(port.clone(), self.clock.clone(), self.ttl);
        slots.insert(
            cluster.clone(),
            Slot {
                port,
                service: service.clone(),
            },
        );
        Some(service)
    }

    /// Forgets the cached review of each of `namespaces` of `cluster` (the user switched to
    /// them), so the next ask is fresh. Does nothing for a cluster without a service yet.
    pub fn invalidate_namespaces<'a>(
        &self,
        cluster: &ClusterId,
        namespaces: impl IntoIterator<Item = &'a str>,
    ) {
        let service = self
            .slots
            .lock()
            .get(cluster)
            .map(|slot| slot.service.clone());
        if let Some(service) = service {
            for namespace in namespaces {
                service.invalidate_namespace(Some(namespace));
            }
        }
    }

    /// Forgets `cluster`'s service (the cluster was closed).
    pub fn forget(&self, cluster: &ClusterId) {
        self.slots.lock().remove(cluster);
    }

    /// How many clusters hold a service.
    pub fn len(&self) -> usize {
        self.slots.lock().len()
    }

    /// Whether no cluster holds a service.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The loop that drops the service (and the port it pins) of every cluster `sessions` closes
    /// or disconnects, so a closed cluster's client pool and caches are released. Subscribes
    /// now; spawns nothing: the host runs the future for the life of the app. It ends when the
    /// registry is dropped.
    pub fn follow(&self, sessions: &ClusterSessionManager) -> BoxFuture<'static, ()> {
        let mut updates = sessions.subscribe();
        let sessions = sessions.clone();
        let slots = Arc::downgrade(&self.slots);
        async move {
            while let Some(item) = updates.next().await {
                let stale = match &item {
                    Ok(update) => match &update.change {
                        SessionChange::Closed => true,
                        SessionChange::StateChanged { from, state } => {
                            from.is_connected() && !state.phase().is_connected()
                        }
                        _ => false,
                    },
                    // Updates were missed: look at every cluster.
                    Err(_lagged) => true,
                };
                if stale && !prune(&slots, &sessions) {
                    return;
                }
            }
        }
        .boxed()
    }
}

/// Drops every slot whose cluster is no longer open with the same port. `false` once the
/// registry is gone.
fn prune(slots: &Weak<Slots>, sessions: &ClusterSessionManager) -> bool {
    let Some(slots) = slots.upgrade() else {
        return false;
    };
    // Dropped outside the lock: a port's drop may close connections.
    let mut released = Vec::new();
    slots.lock().retain(|cluster, slot| {
        let current = sessions.get(cluster).and_then(|session| session.access());
        let live = current.is_some_and(|port| Arc::ptr_eq(&port, &slot.port));
        if !live {
            released.push(slot.port.clone());
        }
        live
    });
    drop(released);
    true
}
