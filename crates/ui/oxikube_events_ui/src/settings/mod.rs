//! The `events` settings: how many events each cluster keeps (E14-S02) and the opt-in for
//! desktop notifications (`os_notifications`, E14-S06; see the private `os` module).
//!
//! ```json
//! "events": {
//!   "buffer": 10000,
//!   "os_notifications": false,
//!   "watched_kinds": ["Pod", "Deployment", "StatefulSet", "DaemonSet", "ReplicaSet", "Job",
//!                     "CronJob", "Node", "PersistentVolumeClaim"],
//!   "ignore_reasons": [],
//!   "rate_limit": { "per_object": "5m", "per_reason": 3, "group_window": "1m",
//!                   "max_raised": 5, "raise_window": "30s" },
//!   "warning_window": "1h"
//! }
//! ```
//!
//! The default lives in `assets/settings/default.json` (with comments), the schema is generated
//! from [`EventsContent`] (`settings.schema.json`). `clusters.<id>.events.buffer` gives one
//! cluster its own bound. A value out of range is clamped with a warning in the log, never
//! refused. A change applies at once ([`follow_settings`]): a smaller bound drops the oldest
//! events, and the cluster's feed is opened again with the new bound. `warning_window` is how
//! long a `Warning` event keeps its object's badge in the tables (`30m`, `1h`, `2d`; between one
//! minute and seven days); a value that does not parse falls back to the default with a warning.
//! The notification keys
//! (`watched_kinds`, `ignore_reasons`, `rate_limit`, `os_notifications`; see the private `notify`
//! module) become the notification centre's rules ([`follow_notification_rules`]), the first
//! three of them also per cluster.

mod follow;
mod notify;
#[cfg(test)]
mod notify_tests;
mod os;
#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::time::Duration;

use gpui::App;
use oxikube_app::events::{
    DEFAULT_EVENTS_BUFFER, EventsConfig, MAX_EVENTS_BUFFER, MIN_EVENTS_BUFFER,
};
use oxikube_app::notifications::DEFAULT_WATCHED_KINDS;
use oxikube_app::warning_badges::{DEFAULT_WARNING_WINDOW, clamp_warning_window};
use oxikube_domain::ids::ClusterId;
use oxikube_settings::{Settings, SettingsStore};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub use follow::{follow_notification_rules, follow_settings, follow_warning_window};
pub use notify::{MAX_IGNORED_REASONS, MAX_WATCHED_KINDS, RateLimit, RateLimitContent};

/// What one settings layer says about events: the `events` object of `settings.json`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct EventsContent {
    /// Events each cluster keeps in memory (100 to 50,000; out of range is clamped), newest by
    /// last seen; older ones are dropped. A full buffer of 10,000 holds about 17 MB per cluster
    /// (the events and the watch's index of them). A change applies at once; set it under
    /// `clusters.<id>.events` to give one cluster its own bound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 100, max = 50_000))]
    pub buffer: Option<i64>,
    /// Show Warning notifications on the desktop as well as in Oxikube (opt-in; default off).
    /// Turning this on asks the OS for permission where it has such a thing. Notifications
    /// carry the same redacted text as the in-app ones and never a secret. It is a per-user
    /// choice about the desktop: `false` under `clusters.<id>.events` keeps that cluster off the
    /// desktop (it still notifies in the app), `true` there cannot turn the desktop on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os_notifications: Option<bool>,
    /// How long a `Warning` event keeps its object's badge in the tables, and the age of the
    /// warnings the overviews list: a number and a unit, `m`inutes, `h`ours or `d`ays (`30m`,
    /// `1h`, `2d`); between 1 minute and 7 days, else the nearest. A change applies at once, to
    /// every cluster (a value under `clusters.<id>.events` is not read).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(regex(pattern = r"^\s*\d+\s*[mhd]\s*$"))]
    pub warning_window: Option<String>,
    /// The kinds whose `Warning` events notify (`Pod`, `Deployment`; case does not matter):
    /// a toast, an entry in the notification centre and, when `os_notifications` is on, a
    /// desktop notification. Defaults to the workload kinds, their pods, nodes and volume claims;
    /// `[]` notifies about nothing. At most 64 kinds. A change applies at once; set it under
    /// `clusters.<id>.events` to watch other kinds in one cluster.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(max = 64), inner(length(min = 1)))]
    pub watched_kinds: Option<Vec<String>>,
    /// Reasons that never notify (`BackOff`, `Unhealthy`; matched exactly, case counts), whatever
    /// the kind. At most 256. A change applies at once; set it under `clusters.<id>.events` to
    /// ignore other reasons in one cluster.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(max = 256), inner(length(min = 1)))]
    pub ignore_reasons: Option<Vec<String>>,
    /// How a burst of Warning events is kept from becoming spam: a notification per object at
    /// most every `per_object`, the objects of a reason folding into one group after
    /// `per_reason`, and a cap on toasts per `raise_window`. One per app; a change applies at
    /// once. See the keys of the object.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_limit: Option<RateLimitContent>,
}

/// The resolved `events` settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventsSettings {
    /// Events each cluster keeps, already clamped.
    pub buffer: usize,
    /// Whether desktop (OS) notifications are on (E14-S06).
    pub os_notifications: bool,
    /// How long a warning keeps its badge, already parsed and clamped.
    pub warning_window: Duration,
    /// The kinds whose `Warning` events notify, trimmed and de-duplicated (E14-S05).
    pub watched_kinds: Vec<Arc<str>>,
    /// The reasons that never notify.
    pub ignore_reasons: Vec<Arc<str>>,
    /// The notification rate limit, already parsed and clamped.
    pub rate_limit: RateLimit,
}

impl Settings for EventsSettings {
    const KEY: Option<&'static str> = Some("events");
    type Content = EventsContent;

    fn from_content(content: EventsContent) -> Self {
        Self {
            buffer: clamp(content.buffer),
            os_notifications: content.os_notifications.unwrap_or(false),
            warning_window: parse_window(content.warning_window.as_deref()),
            watched_kinds: notify::names(
                "events.watched_kinds",
                content.watched_kinds.as_deref(),
                &DEFAULT_WATCHED_KINDS,
                true,
                MAX_WATCHED_KINDS,
            ),
            ignore_reasons: notify::names(
                "events.ignore_reasons",
                content.ignore_reasons.as_deref(),
                &[],
                false,
                MAX_IGNORED_REASONS,
            ),
            rate_limit: RateLimit::from_content(&content.rate_limit.unwrap_or_default()),
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

/// `1h`, `30m`, `2d` as a duration within the allowed range; the default for nothing or text
/// that does not parse (with a warning).
fn parse_window(text: Option<&str>) -> Duration {
    let Some(text) = text else {
        return DEFAULT_WARNING_WINDOW;
    };
    let trimmed = text.trim();
    let parsed = trimmed
        .split_at_checked(trimmed.len().saturating_sub(1))
        .and_then(|(number, unit)| {
            let number: u64 = number.trim().parse().ok()?;
            let unit = match unit {
                "m" => 60,
                "h" => 3_600,
                "d" => 86_400,
                _ => return None,
            };
            Some(Duration::from_secs(number.checked_mul(unit)?))
        });
    match parsed {
        Some(window) => {
            let clamped = clamp_warning_window(window);
            if clamped != window {
                tracing::warn!(
                    key = "events.warning_window",
                    value = text,
                    "setting out of range (1m to 7d): using the nearest allowed value"
                );
            }
            clamped
        }
        None => {
            tracing::warn!(
                key = "events.warning_window",
                value = text,
                "not a duration like 30m, 1h or 2d: using the default"
            );
            DEFAULT_WARNING_WINDOW
        }
    }
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

    /// The warning window the settings in `cx` say (the default without a store).
    fn window_in(cx: &App) -> Duration {
        cx.try_global::<SettingsStore>()
            .and_then(|store| store.try_get::<Self>(None))
            .map_or(DEFAULT_WARNING_WINDOW, |s| s.warning_window)
    }

    /// [`config`](Self::config) from the app's store, or `base` without one.
    fn config_in(cx: &App, base: EventsConfig) -> EventsConfig {
        match cx.try_global::<SettingsStore>() {
            Some(store) => Self::config(store, base),
            None => base,
        }
    }
}
