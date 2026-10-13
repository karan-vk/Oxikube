//! The `events` settings: how many events each cluster keeps (E14-S02) and the opt-in for
//! desktop notifications (`os_notifications`, E14-S06; see [`os`]).
//!
//! ```json
//! "events": {
//!   "buffer": 10000,
//!   "os_notifications": false
//! }
//! ```
//!
//! The default lives in `assets/settings/default.json` (with comments), the schema is generated
//! from [`EventsContent`] (`settings.schema.json`). `clusters.<id>.events.buffer` gives one
//! cluster its own bound. A value out of range is clamped with a warning in the log, never
//! refused. A change applies at once ([`follow_settings`]): a smaller bound drops the oldest
//! events, and the cluster's feed is opened again with the new bound.

mod follow;
mod os;
#[cfg(test)]
mod tests;

use gpui::App;
use oxikube_app::events::{
    DEFAULT_EVENTS_BUFFER, EventsConfig, MAX_EVENTS_BUFFER, MIN_EVENTS_BUFFER,
};
use oxikube_domain::ids::ClusterId;
use oxikube_settings::{Settings, SettingsStore};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub use follow::follow_settings;

/// What one settings layer says about events: the `events` object of `settings.json`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct EventsContent {
    /// Events each cluster keeps in memory (100 to 50,000; out of range is clamped), newest by
    /// last seen; older ones are dropped. A full buffer of 10,000 holds about 11 MB. A change
    /// applies at once; set it under `clusters.<id>.events` to give one cluster its own bound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 100, max = 50_000))]
    pub buffer: Option<i64>,
    /// Show Warning notifications on the desktop as well as in Oxikube (opt-in; default off).
    /// Turning this on asks the OS for permission where it has such a thing. Notifications
    /// carry the same redacted text as the in-app ones and never a secret. It is a per-user
    /// choice about the desktop: a `clusters.<id>.events` block does not change it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os_notifications: Option<bool>,
}

/// The resolved `events` settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EventsSettings {
    /// Events each cluster keeps, already clamped.
    pub buffer: usize,
    /// Whether desktop (OS) notifications are on (E14-S06).
    pub os_notifications: bool,
}

impl Settings for EventsSettings {
    const KEY: Option<&'static str> = Some("events");
    type Content = EventsContent;

    fn from_content(content: EventsContent) -> Self {
        Self {
            buffer: clamp(content.buffer),
            os_notifications: content.os_notifications.unwrap_or(false),
        }
    }
}

impl Default for EventsSettings {
    /// What the shipped `default.json` says.
    fn default() -> Self {
        Self::from_content(EventsContent::default())
    }
}

oxikube_settings::register_settings!(EventsSettings);

/// `value` (or the default when unset) within the allowed range, with a warning when clamped.
fn clamp(value: Option<i64>) -> usize {
    let Some(value) = value else {
        return DEFAULT_EVENTS_BUFFER;
    };
    let (min, max) = (MIN_EVENTS_BUFFER as i64, MAX_EVENTS_BUFFER as i64);
    let clamped = value.clamp(min, max);
    if clamped != value {
        tracing::warn!(
            key = "events.buffer",
            value,
            clamped,
            "setting out of range ({min} to {max}): using the nearest allowed value"
        );
    }
    clamped as usize
}

impl EventsSettings {
    /// The `EventService` config the settings in `store` say: the global bound and each cluster
    /// that sets its own (keys of `clusters` that are not cluster ids are skipped), over `base`'s
    /// timings.
    pub fn config(store: &SettingsStore, base: EventsConfig) -> EventsConfig {
        let buffer = store
            .try_get::<Self>(None)
            .map_or(DEFAULT_EVENTS_BUFFER, |s| s.buffer);
        let cluster_buffers = store
            .cluster_values::<Self>()
            .filter_map(|(id, s)| Some((id.parse::<ClusterId>().ok()?, s.buffer)))
            .filter(|(_, b)| *b != buffer)
            .collect();
        EventsConfig {
            buffer,
            cluster_buffers,
            ..base
        }
    }

    /// [`config`](Self::config) from the app's store, or `base` without one.
    fn config_in(cx: &App, base: EventsConfig) -> EventsConfig {
        match cx.try_global::<SettingsStore>() {
            Some(store) => Self::config(store, base),
            None => base,
        }
    }
}
