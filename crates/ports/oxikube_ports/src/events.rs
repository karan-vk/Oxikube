//! [`EventsPort`]: the cluster's Kubernetes `Event`s as one live feed of domain
//! [`Event`]s (E14-S01).
//!
//! # Adapter
//!
//! Implemented by `oxikube_kube::KubeEvents`; the testkit ships `FakeEventsPort` and the
//! scripted `FakeEventFeed`. The `EventService` of `oxikube_app` (E14-S02) is the consumer:
//! it opens **one** cluster-wide feed per cluster and every view, badge, notification and
//! tool reads from its index, never from a watch of its own.
//!
//! # Contract
//!
//! [`watch_events`](EventsPort::watch_events) returns an [`EventsFeed`]: a [`WatchFeed`] of
//! domain [`Event`]s, batched like any other feed (the first batch is one `Restarted` with the
//! events held, later batches carry `Applied` / `Deleted` deltas), plus an
//! [`EventsMonitor`] the consumer polls for [`EventsStatus`]. Dropping the feed stops the
//! watches (abort-on-drop).
//!
//! Kubernetes serves the same stored events on two APIs, `core/v1` and `events.k8s.io/v1`,
//! with different field names. The adapter normalises both into the domain
//! [`Event`] (`Event::from_core_v1` / `from_events_v1`) and, with [`EventApiChoice::Auto`],
//! watches **exactly one** of them: `events.k8s.io/v1` when the cluster serves it, else
//! `core/v1`. [`EventsStatus::apis`] says which.
//!
//! # Scope and RBAC
//!
//! The [`WatchScope`] is a cluster-wide watch or one watch per namespace of a set. When a
//! cluster-wide watch is refused (HTTP 403) the adapter falls back to per-namespace watches
//! over [`EventsWatchOptions::fallback_namespaces`] (when given) and reports which scopes the
//! user may not read in [`EventsStatus::forbidden`], so the UI can say "events in `prod` are
//! hidden: no permission" instead of showing an empty list.
//!
//! The text of an event is whatever the cluster wrote; nothing here adds secrets.

use std::sync::Arc;

use async_trait::async_trait;
use oxikube_domain::OxiResult;
use oxikube_domain::event::Event;
use oxikube_domain::ids::Gvk;
use oxikube_domain::session::WatchScope;

use crate::feed::WatchFeed;

/// The two APIs that serve Kubernetes events. Both are views of the same stored objects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventApi {
    /// `core/v1` `Event` (`involvedObject`, `message`, `count`, `lastTimestamp`).
    Core,
    /// `events.k8s.io/v1` `Event` (`regarding`, `note`, `series`, `eventTime`).
    EventsV1,
}

impl EventApi {
    /// The kind this API serves.
    pub fn gvk(self) -> Gvk {
        match self {
            Self::Core => Gvk::new("", "v1", "Event"),
            Self::EventsV1 => Gvk::new("events.k8s.io", "v1", "Event"),
        }
    }
}

/// Which event API a feed watches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EventApiChoice {
    /// One API per cluster: `events.k8s.io/v1` when discovery serves it (and the user may read
    /// it), else `core/v1`. The default.
    #[default]
    Auto,
    /// Only `core/v1`: forces the fallback, for example to test against a cluster without
    /// `events.k8s.io` or for a user whose RBAC covers only the core group.
    CoreOnly,
    /// Only `events.k8s.io/v1`.
    EventsV1Only,
    /// Both APIs, merged and de-duplicated by uid. Double the server work for the same
    /// events; kept for diagnostics and for tests of the merge.
    Both,
}

/// What a feed watches besides its scope.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EventsWatchOptions {
    /// Which API to watch. Default [`EventApiChoice::Auto`].
    pub api: EventApiChoice,
    /// Only events about the object with this `uid` (server-side field selector, filtered
    /// again on the client). The `EventService` leaves it `None`: one feed per cluster.
    pub regarding_uid: Option<String>,
    /// Most events held for this feed instead of the adapter's configured capacity.
    pub capacity: Option<usize>,
    /// Namespaces to watch one by one when a [`WatchScope::Cluster`] watch is forbidden
    /// (the user's namespace selection, or the kubeconfig context's namespace). Empty: a
    /// refused cluster-wide watch is the feed's final error.
    pub fallback_namespaces: Vec<String>,
}

impl EventsWatchOptions {
    /// Options for the events of the object with `uid`.
    pub fn for_object(uid: impl Into<String>) -> Self {
        Self {
            regarding_uid: Some(uid.into()),
            ..Self::default()
        }
    }

    /// Sets the capacity of this feed.
    #[must_use]
    pub fn capacity(mut self, capacity: usize) -> Self {
        self.capacity = Some(capacity);
        self
    }

    /// Chooses the API to watch.
    #[must_use]
    pub fn api(mut self, api: EventApiChoice) -> Self {
        self.api = api;
        self
    }

    /// Sets the namespaces to watch when a cluster-wide watch is forbidden.
    #[must_use]
    pub fn fallback_namespaces(mut self, namespaces: impl IntoIterator<Item = String>) -> Self {
        self.fallback_namespaces = namespaces.into_iter().collect();
        self
    }
}

/// A part of the cluster whose events the user may not read.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ForbiddenScope {
    /// The cluster-wide watch was refused and no namespace could be watched instead.
    ClusterWide,
    /// The watch of this namespace was refused.
    Namespace(String),
}

/// A point-in-time reading of a running events feed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EventsStatus {
    /// The APIs being watched now: one for [`EventApiChoice::Auto`].
    pub apis: Vec<EventApi>,
    /// The most events the feed holds.
    pub capacity: usize,
    /// Events held now.
    pub len: usize,
    /// Events dropped to stay within `capacity`, since the feed opened.
    pub evicted: u64,
    /// Objects the server sent that did not map to an event, since the feed opened.
    pub skipped: u64,
    /// Scopes the user may not read (sorted, unique). Empty when everything is readable.
    pub forbidden: Vec<ForbiddenScope>,
}

/// Reads the live [`EventsStatus`] of a feed. Cheap and non-blocking.
pub trait EventsMonitor: Send + Sync {
    /// The status now.
    fn status(&self) -> EventsStatus;
}

/// A live events feed and its monitor. See the [module docs](self).
pub struct EventsFeed {
    /// Coalesced deltas of normalised events.
    pub feed: WatchFeed<Event>,
    /// The feed's status, polled by the consumer.
    pub monitor: Arc<dyn EventsMonitor>,
}

impl std::fmt::Debug for EventsFeed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventsFeed")
            .field("status", &self.monitor.status())
            .finish_non_exhaustive()
    }
}

/// Opens the events feed of one cluster.
///
/// # Effects
///
/// Read-only: lists and watches events.
///
/// # Errors
///
/// [`Validation`](oxikube_domain::ErrorKind::Validation) for an empty namespace list;
/// [`Unsupported`](oxikube_domain::ErrorKind::Unsupported) when the cluster serves none of the
/// requested APIs. Failures after the feed opened arrive on the feed: retryable ones as `Err`
/// items while it backs off, a final one (for example every scope forbidden) as the last item.
#[async_trait]
pub trait EventsPort: Send + Sync {
    /// Opens a feed of the events in `scope`.
    async fn watch_events(
        &self,
        scope: &WatchScope,
        options: &EventsWatchOptions,
    ) -> OxiResult<EventsFeed>;
}
