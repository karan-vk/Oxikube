//! Settings and per-feed options of the events feed.

use oxikube_domain::ids::Gvk;

/// Events a feed keeps when [`EventsConfig::capacity`] is not set: a few thousand, enough for
/// an hour of a busy namespace and a few MB of memory.
pub const DEFAULT_EVENT_CAPACITY: usize = 5_000;

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

    /// This API's bit in an entry's holder mask.
    pub(super) fn bit(self) -> u8 {
        match self {
            Self::Core => 0b01,
            Self::EventsV1 => 0b10,
        }
    }

    /// The field selector that restricts this API to the events of the object with `uid`.
    /// The two APIs name the field differently, and each rejects the other's name.
    pub(super) fn uid_selector(self, uid: &str) -> String {
        match self {
            Self::Core => format!("involvedObject.uid={uid}"),
            Self::EventsV1 => format!("regarding.uid={uid}"),
        }
    }
}

/// Which of the two event APIs a feed watches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EventApis {
    /// Both, merged and de-duplicated. An API the cluster does not serve, or that is
    /// forbidden to the user, is skipped as long as the other works.
    #[default]
    Both,
    /// Only `core/v1`, for example to test against a cluster without `events.k8s.io`.
    CoreOnly,
    /// Only `events.k8s.io/v1`.
    EventsV1Only,
}

impl EventApis {
    pub(super) fn list(self) -> &'static [EventApi] {
        match self {
            Self::Both => &[EventApi::Core, EventApi::EventsV1],
            Self::CoreOnly => &[EventApi::Core],
            Self::EventsV1Only => &[EventApi::EventsV1],
        }
    }
}

/// Settings of the feeds opened from one [`KubeEvents`](super::KubeEvents).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventsConfig {
    /// Most events a feed holds. When a newer event arrives at capacity the oldest by last-seen
    /// time is evicted (and reported). Default [`DEFAULT_EVENT_CAPACITY`]; at least 1.
    pub capacity: usize,
    /// Which APIs to watch. Default [`EventApis::Both`].
    pub apis: EventApis,
}

impl Default for EventsConfig {
    fn default() -> Self {
        Self {
            capacity: DEFAULT_EVENT_CAPACITY,
            apis: EventApis::Both,
        }
    }
}

/// What one feed watches besides its scope.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EventsOptions {
    /// Only events about the object with this `uid`. Applied as a server-side field selector
    /// on each API and again on the client, so a server that ignores or rejects the selector
    /// still yields a correct feed.
    pub regarding_uid: Option<String>,
    /// Capacity for this feed instead of [`EventsConfig::capacity`].
    pub capacity: Option<usize>,
}

impl EventsOptions {
    /// Options for the events of the object with `uid`.
    pub fn for_object(uid: impl Into<String>) -> Self {
        Self {
            regarding_uid: Some(uid.into()),
            capacity: None,
        }
    }

    /// Sets the capacity of this feed.
    #[must_use]
    pub fn capacity(mut self, capacity: usize) -> Self {
        self.capacity = Some(capacity);
        self
    }
}
