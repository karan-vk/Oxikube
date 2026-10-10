//! Following the cluster's kinds while a session is connected (E03-F544).
//!
//! On connect the manager starts [`DiscoveryPort::subscribe`]: the adapter watches
//! `CustomResourceDefinition`s, re-runs discovery and reports. Each report becomes a
//! [`SessionChange`] (`KindsChanged`, `CrdWatchChanged`) so the sidebar, the kind pickers and
//! anything else that follows the session see a CRD appear or disappear without reconnecting,
//! and see when the watch is refused (`Forbidden`) instead of finding out by absence.
//!
//! A kinds change also invalidates the connection's [`SchemaPort`] (E10-S01), before the change is
//! announced, so a listener that asks for a schema in response reads the server's current one.
//!
//! The forwarder is the one task the manager spawns. It belongs to the session entry
//! ([`KindWatch`]): releasing the connection (disconnect, close, reconnect, a connection that
//! went to `Error`) aborts it, which drops the subscription and with it the adapter's watch.

use std::sync::{Arc, Weak};

use futures::StreamExt as _;
use oxikube_domain::ids::ClusterId;
use oxikube_ports::{DiscoveryEvent, DiscoveryEvents, DiscoveryPort, SchemaPort};
use tokio::runtime::Handle;
use tokio::task::JoinHandle;

use super::manager::Shared;
use super::updates::SessionChange;

/// Starts the subscription when a Tokio runtime is running (the adapter's watch needs one);
/// without one the session simply does not follow kinds.
pub(super) fn subscribe(discovery: &dyn DiscoveryPort) -> Option<DiscoveryEvents> {
    Handle::try_current().ok().map(|_| discovery.subscribe())
}

/// The forwarder task of one connection; aborted when dropped.
#[derive(Debug)]
pub(super) struct KindWatch(JoinHandle<()>);

impl Drop for KindWatch {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl Shared {
    /// Forwards `events` into the session's updates until the connection of `generation` goes
    /// away. `None` when no runtime is running.
    pub(super) fn follow_kinds(
        self: &Arc<Self>,
        cluster: &ClusterId,
        generation: u64,
        mut events: DiscoveryEvents,
        schemas: Arc<dyn SchemaPort>,
    ) -> Option<KindWatch> {
        let handle = Handle::try_current().ok()?;
        let shared = Arc::downgrade(self);
        let cluster = cluster.clone();
        Some(KindWatch(handle.spawn(async move {
            while let Some(event) = events.next().await {
                if matches!(event, DiscoveryEvent::KindsChanged(_)) {
                    // Local bookkeeping only; a failure cannot matter to the session.
                    let _ = schemas.invalidate(&cluster).await;
                }
                if !forward(&shared, &cluster, generation, event) {
                    break;
                }
            }
        })))
    }
}

/// Applies one event; `false` ends the forwarder (the manager or the connection is gone).
fn forward(
    shared: &Weak<Shared>,
    cluster: &ClusterId,
    generation: u64,
    event: DiscoveryEvent,
) -> bool {
    let Some(shared) = shared.upgrade() else {
        return false;
    };
    let Some(entry) = shared.entry(cluster) else {
        return false;
    };
    let mut e = entry.lock();
    if e.generation != generation || !e.phase().is_connected() {
        return false;
    }
    match event {
        DiscoveryEvent::KindsChanged(change) => {
            shared
                .updates
                .send(cluster, SessionChange::KindsChanged(change));
        }
        DiscoveryEvent::CrdWatch(status) => e.set_crd_watch(status, &shared.updates),
    }
    true
}
