//! [`BadgeTrack`]: the task that keeps one cluster's badges current.

use futures::StreamExt as _;
use oxikube_domain::ids::ClusterId;

use super::WarningBadges;
use crate::events::{EventFilter, EventsChanged};
use crate::store::{TaskGuard, spawn_guarded};

/// Keeps a cluster's [`ClusterBadges`](super::ClusterBadges) current while it lives: a
/// warnings-only subscription to the service (which runs the cluster's one events feed) and the
/// task that applies what it reports. Dropping it stops both; the badges stay as they were.
pub struct BadgeTrack {
    _task: TaskGuard,
}

impl std::fmt::Debug for BadgeTrack {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BadgeTrack").finish_non_exhaustive()
    }
}

impl BadgeTrack {
    pub(super) fn start(badges: &WarningBadges, cluster: &ClusterId) -> Self {
        let mut subscription = badges
            .shared
            .service
            .subscribe(cluster, EventFilter::warnings());
        let handle = badges.clone();
        let tracked = cluster.clone();
        let task = spawn_guarded(&badges.shared.spawner, async move {
            while let Some(change) = subscription.next().await {
                handle.apply(&tracked, change);
            }
        });
        Self { _task: task }
    }
}

impl WarningBadges {
    /// Applies one report of the warnings subscription to `cluster`'s badges and wakes the
    /// views when they changed.
    pub(super) fn apply(&self, cluster: &ClusterId, change: EventsChanged) {
        let badges = self.for_cluster(cluster);
        let service = &self.shared.service;
        let mut changed = false;
        if change.reset {
            let held = service
                .read(cluster, |buffer| {
                    buffer.warnings().cloned().collect::<Vec<_>>()
                })
                .unwrap_or_default();
            badges.replace(&held);
            changed = true;
        } else {
            let events = change.added.into_iter().chain(change.updated);
            let (applied, lost) = badges.apply(&change.removed, events);
            changed |= applied;
            // An object that lost the warning it showed may have an older one.
            for uid in lost {
                let filter = EventFilter::warnings().object(uid.clone());
                let next = service
                    .read(cluster, |buffer| buffer.query(&filter).next().cloned())
                    .flatten();
                if let Some(event) = next {
                    badges.offer(&event);
                }
            }
        }
        if changed {
            self.shared.epoch.bump();
        }
    }
}
