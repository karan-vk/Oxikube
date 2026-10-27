//! [`AccessRegistry`]: one [`AccessService`] per connected cluster.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use oxikube_domain::ids::ClusterId;
use oxikube_ports::{AccessReviewPort, ClockPort};
use parking_lot::Mutex;

use super::service::{AccessService, DEFAULT_TTL};
use crate::session::ClusterSession;

struct Slot {
    port: Arc<dyn AccessReviewPort>,
    service: AccessService,
}

/// Hands out the [`AccessService`] of each connected cluster and starts a fresh one when the
/// cluster reconnects (a new connection has a new port), which is the "invalidate on connect and
/// context switch" rule. Cheap to clone; clones share the services.
#[derive(Clone)]
pub struct AccessRegistry {
    clock: Arc<dyn ClockPort>,
    ttl: Duration,
    slots: Arc<Mutex<HashMap<ClusterId, Slot>>>,
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
}
