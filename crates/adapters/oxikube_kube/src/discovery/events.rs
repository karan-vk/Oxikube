//! [`DiscoveryPort::subscribe`]: the registry's changes and the CRD watch status as one stream.
//!
//! The stream owns the [`CrdWatch`](super::CrdWatch) it started, so dropping it (the session was
//! released) aborts the watch.

use futures::StreamExt as _;
use futures::stream;
use oxikube_domain::ids::Gvk;
use oxikube_domain::kinds::ResourceKind;
use oxikube_ports::{CrdWatchStatus, DiscoveryEvent, DiscoveryEvents, KindsChange};
use tokio::sync::{broadcast, watch};

use super::{CrdWatch, CrdWatchConfig, KubeDiscovery, RegistryDiff};

/// What the stream owns while it is polled.
struct Follow {
    diffs: broadcast::Receiver<std::sync::Arc<RegistryDiff>>,
    status: watch::Receiver<CrdWatchStatus>,
    /// Keeps the watch running; aborted when the stream is dropped.
    _watch: CrdWatch,
}

impl KubeDiscovery {
    /// Starts a CRD watch (default tuning) and returns its events.
    pub(super) fn events(&self) -> DiscoveryEvents {
        // Receivers first, so nothing the watch publishes is missed.
        let diffs = self.registry_changes();
        let mut status = self.shared.crd_status.subscribe();
        // A refusal left by an earlier watch is news to this subscriber; `Watching` is the
        // assumed state and is only announced when it comes back after a refusal.
        let initial = status.borrow_and_update().clone();
        let first = initial
            .is_forbidden()
            .then(|| DiscoveryEvent::CrdWatch(initial));
        let follow = Follow {
            diffs,
            status,
            _watch: self.watch_crds(CrdWatchConfig::default()),
        };
        stream::iter(first)
            .chain(stream::unfold(follow, |mut follow| async move {
                let event = tokio::select! {
                    diff = follow.diffs.recv() => match diff {
                        Ok(diff) => kinds_changed(&diff),
                        // Missed some: an empty change tells the subscriber to re-read.
                        Err(broadcast::error::RecvError::Lagged(_)) => {
                            DiscoveryEvent::KindsChanged(KindsChange::default())
                        }
                        Err(broadcast::error::RecvError::Closed) => return None,
                    },
                    changed = follow.status.changed() => {
                        if changed.is_err() {
                            return None;
                        }
                        DiscoveryEvent::CrdWatch(follow.status.borrow_and_update().clone())
                    }
                };
                Some((event, follow))
            }))
            .boxed()
    }
}

fn kinds_changed(diff: &RegistryDiff) -> DiscoveryEvent {
    let gvks =
        |kinds: &[ResourceKind]| -> Vec<Gvk> { kinds.iter().map(|k| k.gvk.clone()).collect() };
    DiscoveryEvent::KindsChanged(KindsChange {
        added: gvks(&diff.added),
        removed: gvks(&diff.removed),
        changed: diff.changed.iter().map(|c| c.after.gvk.clone()).collect(),
    })
}
