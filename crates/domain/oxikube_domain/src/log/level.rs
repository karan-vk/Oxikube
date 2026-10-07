//! [`LogLevel`] and [`LevelChip`]: the normalised severity of a structured log line and the
//! chips the log view filters by.

use serde::{Deserialize, Serialize};

/// The severity of a structured (JSON) log line, normalised across loggers: zap, logrus, bunyan
/// and pino all spell it differently (`"info"`, `30`, `"INFO"`, `"warning"`, ...).
///
/// Ordered from least to most severe; [`Unknown`](Self::Unknown) is a structured line with no
/// level field, or one this crate cannot read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    /// Very fine-grained tracing.
    Trace,
    /// Debugging detail.
    Debug,
    /// Routine information.
    Info,
    /// Something worth attention.
    Warn,
    /// A failure.
    Error,
    /// A failure the process does not survive (fatal, panic, critical, alert, emergency).
    Fatal,
    /// No level, or an unrecognised one.
    Unknown,
}

impl LogLevel {
    /// The six real levels, least severe first.
    pub const KNOWN: [LogLevel; 6] = [
        LogLevel::Trace,
        LogLevel::Debug,
        LogLevel::Info,
        LogLevel::Warn,
        LogLevel::Error,
        LogLevel::Fatal,
    ];

    /// Lower-case name: `trace`, `debug`, `info`, `warn`, `error`, `fatal`, `unknown`.
    pub const fn label(self) -> &'static str {
        match self {
            LogLevel::Trace => "trace",
            LogLevel::Debug => "debug",
            LogLevel::Info => "info",
            LogLevel::Warn => "warn",
            LogLevel::Error => "error",
            LogLevel::Fatal => "fatal",
            LogLevel::Unknown => "unknown",
        }
    }

    /// The chip that shows or hides lines of this level: a line without a known level belongs to
    /// the `text` chip.
    pub const fn chip(self) -> LevelChip {
        match self {
            LogLevel::Trace => LevelChip::Trace,
            LogLevel::Debug => LevelChip::Debug,
            LogLevel::Info => LevelChip::Info,
            LogLevel::Warn => LevelChip::Warn,
            LogLevel::Error => LevelChip::Error,
            LogLevel::Fatal => LevelChip::Fatal,
            LogLevel::Unknown => LevelChip::Text,
        }
    }
}

/// One level chip of the log view's filter: the six levels, plus `text` for the lines that are
/// not structured (plain output) or carry no level. Serialised as its label, which is what a
/// `logs::ToggleLevel` tool call passes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LevelChip {
    /// `trace` lines.
    Trace,
    /// `debug` lines.
    Debug,
    /// `info` lines.
    Info,
    /// `warn` lines.
    Warn,
    /// `error` lines.
    Error,
    /// `fatal` lines.
    Fatal,
    /// Plain-text lines and structured lines without a level.
    Text,
}

impl LevelChip {
    /// Every chip, in display order.
    pub const ALL: [LevelChip; 7] = [
        LevelChip::Trace,
        LevelChip::Debug,
        LevelChip::Info,
        LevelChip::Warn,
        LevelChip::Error,
        LevelChip::Fatal,
        LevelChip::Text,
    ];

    /// Lower-case name: `trace`, ..., `fatal`, `text`.
    pub const fn label(self) -> &'static str {
        match self {
            LevelChip::Trace => "trace",
            LevelChip::Debug => "debug",
            LevelChip::Info => "info",
            LevelChip::Warn => "warn",
            LevelChip::Error => "error",
            LevelChip::Fatal => "fatal",
            LevelChip::Text => "text",
        }
    }

    /// The chip's position in [`ALL`](Self::ALL) (a bit index for a set of chips).
    pub const fn index(self) -> u8 {
        self as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_are_ordered_by_severity() {
        assert!(LogLevel::Trace < LogLevel::Info);
        assert!(LogLevel::Warn < LogLevel::Error);
        assert!(LogLevel::Fatal < LogLevel::Unknown);
    }

    #[test]
    fn chips_follow_levels_and_unknown_is_text() {
        for level in LogLevel::KNOWN {
            assert_eq!(level.chip().label(), level.label());
        }
        assert_eq!(LogLevel::Unknown.chip(), LevelChip::Text);
    }

    #[test]
    fn chips_serialise_as_their_label_and_index_matches_all() {
        for (i, chip) in LevelChip::ALL.into_iter().enumerate() {
            assert_eq!(usize::from(chip.index()), i);
            let json = serde_json::to_value(chip).unwrap();
            assert_eq!(json, serde_json::json!(chip.label()));
            assert_eq!(serde_json::from_value::<LevelChip>(json).unwrap(), chip);
        }
        assert!(serde_json::from_value::<LevelChip>(serde_json::json!("loud")).is_err());
    }
}
