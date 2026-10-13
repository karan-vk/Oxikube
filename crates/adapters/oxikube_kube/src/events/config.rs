//! Settings of the events feed; the options of one feed are the port's
//! [`EventsWatchOptions`].

pub use oxikube_ports::EventApi;
use oxikube_ports::{EventApiChoice, EventsWatchOptions};

/// Events a feed keeps when [`EventsConfig::capacity`] is not set: a few thousand, enough for
/// an hour of a busy namespace and a few MB of memory. The app's `EventService` passes its own
/// `events.buffer` setting.
pub const DEFAULT_EVENT_CAPACITY: usize = 5_000;

/// Which of the two event APIs a feed watches (re-exported name of the port's choice).
pub type EventApis = EventApiChoice;

/// What one feed watches besides its scope (the port's options).
pub type EventsOptions = EventsWatchOptions;

/// The adapter-side details of an [`EventApi`]: its holder bit in the ring and its UID field
/// selector.
pub(super) trait EventApiExt {
    /// This API's bit in an entry's holder mask.
    fn bit(self) -> u8;

    /// The field selector that restricts this API to the events of the object with `uid`.
    /// The two APIs name the field differently, and each rejects the other's name.
    fn uid_selector(self, uid: &str) -> String;
}

impl EventApiExt for EventApi {
    fn bit(self) -> u8 {
        match self {
            Self::Core => 0b01,
            Self::EventsV1 => 0b10,
        }
    }

    fn uid_selector(self, uid: &str) -> String {
        match self {
            Self::Core => format!("involvedObject.uid={uid}"),
            Self::EventsV1 => format!("regarding.uid={uid}"),
        }
    }
}

/// The APIs a choice watches, in order of preference.
pub(super) fn preference(choice: EventApiChoice) -> &'static [EventApi] {
    match choice {
        EventApiChoice::Auto | EventApiChoice::Both => &[EventApi::EventsV1, EventApi::Core],
        EventApiChoice::CoreOnly => &[EventApi::Core],
        EventApiChoice::EventsV1Only => &[EventApi::EventsV1],
    }
}

/// Settings of the feeds opened from one [`KubeEvents`](super::KubeEvents).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventsConfig {
    /// Most events a feed holds. When a newer event arrives at capacity the oldest by last-seen
    /// time is evicted (and reported). Default [`DEFAULT_EVENT_CAPACITY`]; at least 1.
    pub capacity: usize,
    /// Which APIs to watch. Default [`EventApiChoice::Auto`]: exactly one per cluster.
    pub apis: EventApiChoice,
}

impl Default for EventsConfig {
    fn default() -> Self {
        Self {
            capacity: DEFAULT_EVENT_CAPACITY,
            apis: EventApiChoice::Auto,
        }
    }
}
