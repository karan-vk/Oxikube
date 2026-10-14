//! Warning badges and the per-object event feed (E14-S04): what a table row's warning icon and a
//! resource's Events tab read, both from the [`EventService`](crate::events::EventService) index
//! and never from a watch of their own.
//!
//! | Piece | Where |
//! |---|---|
//! | the handle: window, per-cluster badges, tracking, the change stream | [`WarningBadges`] (here) |
//! | one cluster's badges: the latest `Warning` per involved object, `O(1)` to read | [`ClusterBadges`] (`cluster`) |
//! | the task that keeps them current while a tab is open | [`BadgeTrack`] (`track`) |
//! | "something changed" for the views, coalesced | [`BadgeChanges`] (`epoch`) |
//! | one object's events for its Events tab | [`ObjectEventsFeed`] (`object_feed`) |
//!
//! # Badges
//!
//! A row shows the badge when its object has a `Warning` event last seen within the window
//! ([`DEFAULT_WARNING_WINDOW`], one hour; `events.warning_window`). The model is a map from the
//! involved object's uid to the newest `Warning` about it, kept up to date from a warnings-only
//! subscription: a table reads one entry per visible row (a hash lookup under a read lock, no
//! scan of events). The window is compared at read time, so a badge expires without any event
//! arriving; [`ClusterBadges::expired_between`] counts the entries that crossed the window between two
//! looks so a view can redraw once when some did. Events the writer gave no involved-object uid are not badged (they stay
//! in the events view).
//!
//! The cluster-wide feed is the service's one: tracking a cluster is a subscription like any
//! other and keeps that feed running while the cluster's tab is open.
//!
//! # Content is data
//!
//! The model holds the events the service holds; nothing here logs a message.

mod cluster;
mod epoch;
mod object_feed;
mod track;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use oxikube_domain::ids::ClusterId;
use oxikube_ports::ClockPort;
use parking_lot::RwLock;

use crate::events::{EventService, seen_at, warning_row};
use crate::overview::{RecentWarnings, WarningRow};
use crate::store::Spawner;

pub use cluster::{ClusterBadges, WarningBadge};
pub use epoch::BadgeChanges;
pub use object_feed::{MAX_OBJECT_EVENTS, ObjectEventsFeed};
pub use track::BadgeTrack;

use epoch::Epoch;

/// How long a `Warning` event keeps its object's badge, when `events.warning_window` says
/// nothing (one hour).
pub const DEFAULT_WARNING_WINDOW: Duration = Duration::from_secs(3_600);

/// The shortest window the setting allows (one minute).
pub const MIN_WARNING_WINDOW: Duration = Duration::from_secs(60);

/// The longest window the setting allows (seven days).
pub const MAX_WARNING_WINDOW: Duration = Duration::from_secs(7 * 24 * 3_600);

/// `window` within [`MIN_WARNING_WINDOW`]..=[`MAX_WARNING_WINDOW`].
pub fn clamp_warning_window(window: Duration) -> Duration {
    window.clamp(MIN_WARNING_WINDOW, MAX_WARNING_WINDOW)
}

/// State shared by every handle, [`ClusterBadges`] and track.
pub(crate) struct Shared {
    pub(crate) service: EventService,
    clock: Arc<dyn ClockPort>,
    spawner: Arc<dyn Spawner>,
    window_secs: Arc<AtomicU64>,
    clusters: RwLock<HashMap<ClusterId, ClusterBadges>>,
    pub(crate) epoch: Epoch,
}

/// The warning badges of every cluster and the per-object event feeds, over one
/// [`EventService`]. Cheap to clone: a handle on shared state.
#[derive(Clone)]
pub struct WarningBadges {
    pub(crate) shared: Arc<Shared>,
}

impl std::fmt::Debug for WarningBadges {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WarningBadges")
            .field("window", &self.window())
            .finish_non_exhaustive()
    }
}

impl WarningBadges {
    /// Badges over `service`, with their pumps on `spawner` (the Tokio bridge), expiring after
    /// `window` on `clock`.
    pub fn new(
        service: EventService,
        clock: Arc<dyn ClockPort>,
        spawner: Arc<dyn Spawner>,
        window: Duration,
    ) -> Self {
        Self {
            shared: Arc::new(Shared {
                service,
                clock,
                spawner,
                window_secs: Arc::new(AtomicU64::new(clamp_warning_window(window).as_secs())),
                clusters: RwLock::new(HashMap::new()),
                epoch: Epoch::default(),
            }),
        }
    }

    /// The events service the badges and the Events tab read.
    pub fn service(&self) -> &EventService {
        &self.shared.service
    }

    /// How long a warning keeps its badge.
    pub fn window(&self) -> Duration {
        Duration::from_secs(self.shared.window_secs.load(Ordering::Relaxed))
    }

    /// Changes the window (`events.warning_window` changed); applies to the next read. Views
    /// listening to [`changes`](Self::changes) are woken.
    pub fn set_window(&self, window: Duration) {
        let secs = clamp_warning_window(window).as_secs();
        if self.shared.window_secs.swap(secs, Ordering::Relaxed) != secs {
            self.shared.epoch.bump();
        }
    }

    /// `cluster`'s badges (made empty on first use). Keep the handle: reading through it takes
    /// no lookup by cluster.
    pub fn for_cluster(&self, cluster: &ClusterId) -> ClusterBadges {
        if let Some(badges) = self.shared.clusters.read().get(cluster) {
            return badges.clone();
        }
        self.shared
            .clusters
            .write()
            .entry(cluster.clone())
            .or_insert_with(|| ClusterBadges::new(self.shared.window_secs.clone()))
            .clone()
    }

    /// Keeps `cluster`'s badges current until the returned [`BadgeTrack`] is dropped. It
    /// subscribes to the cluster's warnings, which runs the cluster's events feed while the
    /// track lives (the tab that owns it is open).
    pub fn track(&self, cluster: &ClusterId) -> BadgeTrack {
        BadgeTrack::start(self, cluster)
    }

    /// A stream that yields once whenever badges changed since it last yielded (a warning
    /// arrived, was removed, or the window changed). Waking is coalesced: a burst is one item.
    pub fn changes(&self) -> BadgeChanges {
        self.shared.epoch.changes()
    }

    /// The feed of the events about one object, for its Events tab. See [`ObjectEventsFeed`].
    pub fn object_feed(
        &self,
        cluster: &ClusterId,
        uid: Option<&str>,
        cluster_scoped_name: Option<&str>,
    ) -> ObjectEventsFeed {
        ObjectEventsFeed::new(&self.shared.service, cluster, uid, cluster_scoped_name)
    }
}

/// The overview's warnings panel over the badges' window: the newest `Warning` events last seen
/// within it (the service's own implementation lists every warning the buffer holds).
impl RecentWarnings for WarningBadges {
    fn latest(&self, cluster: &ClusterId, n: usize) -> Vec<WarningRow> {
        let cutoff = self.shared.clock.now() - self.window();
        self.shared
            .service
            .read(cluster, |buffer| {
                buffer
                    .warnings()
                    .take_while(|event| seen_at(event) >= cutoff)
                    .take(n)
                    .map(|event| warning_row(event))
                    .collect()
            })
            .unwrap_or_default()
    }
}
