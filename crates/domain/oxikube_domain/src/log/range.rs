//! [`LogRange`]: which part of a container's log a log view reads (k9s keys `0` to `6`).

use serde::{Deserialize, Serialize};

/// Which part of a log a view reads: the newest lines, the start of the log, or the lines of the
/// last few minutes. The presets are k9s's: `0` tail, `1` head, `2`-`6` since 1m / 5m / 15m /
/// 30m / 1h.
///
/// Serialised as its short name (`"tail"`, `"head"`, `"1m"`, `"5m"`, `"15m"`, `"30m"`, `"1h"`),
/// which is what a `logs::SetRange` tool call passes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum LogRange {
    /// The newest lines, then follow (the default).
    #[default]
    #[serde(rename = "tail")]
    Tail,
    /// The log from its first line, without following.
    #[serde(rename = "head")]
    Head,
    /// Lines of the last minute, then follow.
    #[serde(rename = "1m")]
    Last1m,
    /// Lines of the last 5 minutes, then follow.
    #[serde(rename = "5m")]
    Last5m,
    /// Lines of the last 15 minutes, then follow.
    #[serde(rename = "15m")]
    Last15m,
    /// Lines of the last 30 minutes, then follow.
    #[serde(rename = "30m")]
    Last30m,
    /// Lines of the last hour, then follow.
    #[serde(rename = "1h")]
    Last1h,
}

impl LogRange {
    /// Every range, in key order (`0` to `6`).
    pub const ALL: [LogRange; 7] = [
        LogRange::Tail,
        LogRange::Head,
        LogRange::Last1m,
        LogRange::Last5m,
        LogRange::Last15m,
        LogRange::Last30m,
        LogRange::Last1h,
    ];

    /// The short name: `tail`, `head`, `1m`, `5m`, `15m`, `30m`, `1h`.
    pub const fn label(self) -> &'static str {
        match self {
            LogRange::Tail => "tail",
            LogRange::Head => "head",
            LogRange::Last1m => "1m",
            LogRange::Last5m => "5m",
            LogRange::Last15m => "15m",
            LogRange::Last30m => "30m",
            LogRange::Last1h => "1h",
        }
    }

    /// The `sinceSeconds` of a since preset; `None` for tail and head.
    pub const fn since_seconds(self) -> Option<i64> {
        match self {
            LogRange::Tail | LogRange::Head => None,
            LogRange::Last1m => Some(60),
            LogRange::Last5m => Some(5 * 60),
            LogRange::Last15m => Some(15 * 60),
            LogRange::Last30m => Some(30 * 60),
            LogRange::Last1h => Some(60 * 60),
        }
    }

    /// Whether the read keeps following new lines (every range but head).
    pub const fn follows(self) -> bool {
        !matches!(self, LogRange::Head)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialises_as_its_label_and_round_trips() {
        for range in LogRange::ALL {
            let json = serde_json::to_value(range).unwrap();
            assert_eq!(json, serde_json::json!(range.label()));
            assert_eq!(serde_json::from_value::<LogRange>(json).unwrap(), range);
        }
        assert!(serde_json::from_value::<LogRange>(serde_json::json!("2h")).is_err());
    }

    #[test]
    fn presets_carry_their_seconds_and_only_head_stops() {
        assert_eq!(LogRange::Tail.since_seconds(), None);
        assert_eq!(LogRange::Head.since_seconds(), None);
        assert_eq!(LogRange::Last15m.since_seconds(), Some(900));
        assert_eq!(LogRange::Last1h.since_seconds(), Some(3_600));
        assert!(!LogRange::Head.follows());
        assert!(LogRange::ALL.iter().filter(|r| r.follows()).count() == 6);
        assert_eq!(LogRange::default(), LogRange::Tail);
    }
}
