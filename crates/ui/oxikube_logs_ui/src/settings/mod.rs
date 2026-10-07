//! The `logs` settings (E08-S01, S10): how much of a stream a view keeps, what it reads first and
//! how it draws it.
//!
//! ```json
//! "logs": {
//!   "buffer_lines": 50000,
//!   "default_tail": 1000,
//!   "wrap": false,
//!   "timestamps": false,
//!   "json_auto_detect": true,
//!   "max_streams": 20
//! }
//! ```
//!
//! The defaults live in `assets/settings/default.json` (with comments), the schema is generated
//! from [`LogsContent`] (`settings.schema.json`). Like every setting the block can be overridden
//! per cluster: `clusters.<id>.logs.buffer_lines` is the usual one (a chatty production
//! cluster), and any other key works the same way.
//!
//! A value out of range is clamped with a warning in the log, never refused: the setting stays
//! usable and the file keeps its other values.
//!
//! | Key | Applies |
//! |---|---|
//! | `buffer_lines` | at once to every open session ([`follow_settings`](crate::follow_settings)) |
//! | `wrap`, `timestamps`, `json_auto_detect` | at once to the open views, without reopening the stream |
//! | `max_streams` | at once to the open multi-pod views (a higher value starts the pods that were left out) |
//! | `default_tail` | to the views opened afterwards (changing it must not reopen what is being read) |
//!
//! k9s's `logger` keys that Oxikube's viewer does not have (`sinceSeconds` as a setting,
//! `disableAutoscroll`, `columnLock`) are not planned: the range presets and the `s` key cover
//! them.

mod clamp;
#[cfg(test)]
mod tests;

use gpui::App;
use oxikube_app::logs::{
    DEFAULT_BUFFER_LINES, DEFAULT_MAX_STREAMS, MAX_BUFFER_LINES, MAX_MAX_STREAMS, MIN_BUFFER_LINES,
};
use oxikube_domain::ids::ClusterId;
use oxikube_settings::{Settings, SettingsLocation, SettingsStore};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use clamp::clamp;
pub use clamp::{MAX_DEFAULT_TAIL, MIN_DEFAULT_TAIL};

/// Lines the tail range reads when `logs.default_tail` says nothing.
pub const DEFAULT_TAIL: u32 = 1_000;

/// What one settings layer says about logs: the `logs` object of `settings.json`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LogsContent {
    /// Lines each log session keeps in memory (100 to 5,000,000; out of range is clamped). When
    /// a chatty pod writes more, the oldest lines are dropped and the viewer shows a "truncated"
    /// marker above the first one. A change applies to open sessions at once; set it under
    /// `clusters.<id>.logs` to give one cluster a different bound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 100, max = 5_000_000))]
    pub buffer_lines: Option<i64>,
    /// Lines the tail range (key `0`) reads before it follows (1 to 100,000; out of range is
    /// clamped). Applies to log views opened afterwards.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1, max = 100_000))]
    pub default_tail: Option<i64>,
    /// Wrap long lines instead of letting them run off the edge (key `w` toggles it per view).
    /// A change applies to open views at once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wrap: Option<bool>,
    /// Show each line's server timestamp before its text (key `t` toggles it per view). A change
    /// applies to open views at once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamps: Option<bool>,
    /// JSON mode (key `j` toggles it per view): render the lines that are JSON objects as level /
    /// time / message columns with expandable fields and the level chips; plain-text lines stay
    /// as they are. A change applies to open views at once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub json_auto_detect: Option<bool>,
    /// Containers a multi-pod log (a Deployment, StatefulSet, DaemonSet, Job or Service) reads at
    /// once (1 to 200). When more match, the first are read in name order and the view says how
    /// many pods were left out. A change applies to open views at once: a higher value starts
    /// the pods that were left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1, max = 200))]
    pub max_streams: Option<i64>,
}

/// The resolved `logs` settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LogsSettings {
    /// Lines each session keeps, already clamped.
    pub buffer_lines: usize,
    /// Lines the tail range reads, already clamped.
    pub default_tail: u32,
    /// Wrap long lines.
    pub wrap: bool,
    /// Show the server timestamp before each line.
    pub timestamps: bool,
    /// JSON mode: render JSON-object lines as columns with expandable fields (E08-S05).
    pub json_auto_detect: bool,
    /// Containers a multi-pod log reads at once, already clamped (E08-S04).
    pub max_streams: usize,
}

impl Settings for LogsSettings {
    const KEY: Option<&'static str> = Some("logs");
    type Content = LogsContent;

    fn from_content(content: LogsContent) -> Self {
        Self {
            buffer_lines: clamp(
                "logs.buffer_lines",
                content.buffer_lines,
                DEFAULT_BUFFER_LINES as i64,
                MIN_BUFFER_LINES as i64,
                MAX_BUFFER_LINES as i64,
            ) as usize,
            default_tail: clamp(
                "logs.default_tail",
                content.default_tail,
                i64::from(DEFAULT_TAIL),
                i64::from(MIN_DEFAULT_TAIL),
                i64::from(MAX_DEFAULT_TAIL),
            ) as u32,
            wrap: content.wrap.unwrap_or(false),
            timestamps: content.timestamps.unwrap_or(false),
            json_auto_detect: content.json_auto_detect.unwrap_or(true),
            max_streams: clamp(
                "logs.max_streams",
                content.max_streams,
                DEFAULT_MAX_STREAMS as i64,
                1,
                MAX_MAX_STREAMS as i64,
            ) as usize,
        }
    }
}

impl Default for LogsSettings {
    /// What the shipped `default.json` says.
    fn default() -> Self {
        Self::from_content(LogsContent::default())
    }
}

impl LogsSettings {
    /// The settings in effect for `cluster` (its `clusters.<id>.logs` overrides over the user's
    /// and the defaults), or the shipped defaults without a settings store.
    pub fn resolve(cluster: &ClusterId, cx: &App) -> Self {
        cx.try_global::<SettingsStore>()
            .and_then(|store| store.try_get::<Self>(Some(SettingsLocation { cluster })))
            .copied()
            .unwrap_or_default()
    }
}

oxikube_settings::register_settings!(LogsSettings);
