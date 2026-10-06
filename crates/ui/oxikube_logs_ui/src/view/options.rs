//! [`ViewOptions`]: what a log view reads (range, container, previous instance) and how it draws
//! it (wrap, timestamps). Plain data; [`ViewOptions::log_options`] is the port's request.

use oxikube_domain::log::LogRange;
use oxikube_ports::{LogOptions, LogSince};

/// Lines the tail range (`0`) reads before following: the newest of the log.
pub const TAIL_LINES: i64 = 1_000;

/// Bytes the head range (`1`) reads from the start of the log (about 10 000 short lines). The
/// head does not follow, so the read ends there and the ring buffer keeps all of it.
pub const HEAD_LIMIT_BYTES: i64 = 1 << 20;

/// The options of one log view. Changing [`range`](Self::range), [`container`](Self::container)
/// or [`previous`](Self::previous) reopens the stream; [`wrap`](Self::wrap) and
/// [`timestamps`](Self::timestamps) only change how the lines already read are drawn.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ViewOptions {
    /// Which part of the log is read: the tail (default), the head or the last minutes.
    pub range: LogRange,
    /// The container read; `None` for the pod's default container.
    pub container: Option<String>,
    /// Read the previous (terminated) instance of the container.
    pub previous: bool,
    /// Wrap long lines (variable row heights) instead of letting them run off the edge.
    pub wrap: bool,
    /// Show each line's server timestamp before its text.
    pub timestamps: bool,
}

impl ViewOptions {
    /// The port's request for these options. Timestamps are always asked of the server: the
    /// viewer shows them on demand without a new read, and the multi-pod merge orders by them.
    /// A previous instance has stopped, so its read never follows.
    pub fn log_options(&self) -> LogOptions {
        let follow = self.range.follows() && !self.previous;
        let mut options = LogOptions {
            container: self.container.clone(),
            follow,
            timestamps: true,
            previous: self.previous,
            ..LogOptions::default()
        };
        match self.range {
            LogRange::Tail => options.tail_lines = Some(TAIL_LINES),
            LogRange::Head => options.limit_bytes = Some(HEAD_LIMIT_BYTES),
            since => options.since = since.since_seconds().map(LogSince::Seconds),
        }
        options
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_reads_the_tail_and_follows() {
        let options = ViewOptions::default().log_options();
        assert!(options.follow && options.timestamps);
        assert_eq!(options.tail_lines, Some(TAIL_LINES));
        assert_eq!(options.since, None);
        assert_eq!(options.limit_bytes, None);
    }

    #[test]
    fn head_reads_from_the_start_and_stops() {
        let options = ViewOptions {
            range: LogRange::Head,
            ..ViewOptions::default()
        }
        .log_options();
        assert!(!options.follow);
        assert_eq!(options.tail_lines, None);
        assert_eq!(options.limit_bytes, Some(HEAD_LIMIT_BYTES));
    }

    #[test]
    fn a_since_preset_reads_its_minutes() {
        let options = ViewOptions {
            range: LogRange::Last30m,
            container: Some("sidecar".into()),
            ..ViewOptions::default()
        }
        .log_options();
        assert!(options.follow);
        assert_eq!(options.since, Some(LogSince::Seconds(1_800)));
        assert_eq!(options.tail_lines, None);
        assert_eq!(options.container.as_deref(), Some("sidecar"));
    }

    #[test]
    fn the_previous_instance_never_follows() {
        let options = ViewOptions {
            previous: true,
            ..ViewOptions::default()
        }
        .log_options();
        assert!(options.previous && !options.follow);
    }
}
