//! The `logs` settings (E08-S01): how much of a stream a session keeps.
//!
//! ```json
//! "logs": { "buffer_lines": 50000 }
//! ```
//!
//! `buffer_lines` bounds every log session's ring buffer: past it the oldest lines are dropped and
//! the viewer shows a "truncated" marker. It is clamped to 100 - 5 000 000 lines. A change applies
//! at once to the sessions that are open ([`follow_settings`](crate::follow_settings)).

use oxikube_app::logs::{DEFAULT_BUFFER_LINES, clamp_buffer_lines};
use oxikube_settings::Settings;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// What one settings layer says about logs: the `logs` object of `settings.json`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct LogsContent {
    /// Lines each log session keeps in memory (100 to 5 000 000). When a chatty pod writes more,
    /// the oldest lines are dropped and the viewer shows a "truncated" marker above the first
    /// one. A change applies to open sessions at once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub buffer_lines: Option<usize>,
}

/// The resolved `logs` settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LogsSettings {
    /// Lines each session keeps, already clamped.
    pub buffer_lines: usize,
}

impl Settings for LogsSettings {
    const KEY: Option<&'static str> = Some("logs");
    type Content = LogsContent;

    fn from_content(content: LogsContent) -> Self {
        Self {
            buffer_lines: clamp_buffer_lines(content.buffer_lines.unwrap_or(DEFAULT_BUFFER_LINES)),
        }
    }
}

oxikube_settings::register_settings!(LogsSettings);

#[cfg(test)]
mod tests {
    use super::*;
    use oxikube_app::logs::{MAX_BUFFER_LINES, MIN_BUFFER_LINES};

    fn lines(value: Option<usize>) -> usize {
        LogsSettings::from_content(LogsContent {
            buffer_lines: value,
        })
        .buffer_lines
    }

    #[test]
    fn unset_is_the_default_and_values_are_clamped() {
        assert_eq!(lines(None), DEFAULT_BUFFER_LINES);
        assert_eq!(lines(Some(2_000)), 2_000);
        assert_eq!(lines(Some(0)), MIN_BUFFER_LINES);
        assert_eq!(lines(Some(usize::MAX)), MAX_BUFFER_LINES);
    }
}
