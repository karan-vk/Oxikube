//! [`follow`]: the session updates the service reacts to.

use std::sync::Weak;

use futures::StreamExt as _;

use super::cluster::Inner;
use crate::session::{SessionChange, SessionUpdates};

/// Reacts to the session updates until the service is dropped: a cluster that connects (or
/// reconnects, with new ports) starts its feed again if it has subscribers, one that disconnects
/// or closes stops its feed and forgets its events, and a new namespace selection reopens a feed
/// that was refused cluster-wide.
pub(crate) async fn follow(inner: Weak<Inner>, mut updates: SessionUpdates) {
    while let Some(item) = updates.next().await {
        let Some(inner) = inner.upgrade() else {
            return;
        };
        match item {
            Ok(update) => match update.change {
                SessionChange::StateChanged { from, state } => {
                    let connected = state.phase().is_connected();
                    if connected && !from.is_connected() {
                        inner.restart(&update.cluster);
                    } else if !connected && from.is_connected() {
                        inner.reconcile(&update.cluster);
                    }
                }
                SessionChange::Closed => inner.reconcile(&update.cluster),
                SessionChange::NamespaceChanged(_) => inner.namespaces_changed(&update.cluster),
                _ => {}
            },
            Err(_lagged) => inner.reconcile_all(),
        }
    }
}
