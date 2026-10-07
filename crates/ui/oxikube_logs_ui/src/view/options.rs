//! [`ViewOptions`]: what a log view reads (range, container, previous instance, tail length,
//! follow) and how it draws it (wrap, timestamps). Plain data; [`ViewOptions::log_options`] is the
//! port's request. [`OpenLogs`] is what `pod::ViewLogs` asks of a view.

use oxikube_domain::log::LogRange;
use oxikube_ports::{LogOptions, LogSince};

/// Lines the tail range (`0`) reads before following: the newest of the log.
pub const TAIL_LINES: i64 = 1_000;

/// Bytes the head range (`1`) reads from the start of the log (about 10 000 short lines). The
/// head does not follow, so the read ends there and the ring buffer keeps all of it.
pub const HEAD_LIMIT_BYTES: i64 = 1 << 20;

/// The options of one log view. Changing what [`log_options`](Self::log_options) asks of the port
/// (range, container, previous, tail length, follow) reopens the stream; [`wrap`](Self::wrap)
/// and [`timestamps`](Self::timestamps) only change how the lines already read are drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewOptions {
    /// Which part of the log is read: the tail (default), the head or the last minutes.
    pub range: LogRange,
    /// The container read; `None` for the pod's default container.
    pub container: Option<String>,
    /// Read the previous (terminated) instance of the container.
    pub previous: bool,
    /// Lines the tail range reads; `None` for [`TAIL_LINES`] (`pod::ViewLogs { tail_lines }`).
    pub tail_lines: Option<u32>,
    /// Keep reading new lines after the range (default `true`; `pod::ViewLogs { follow: false }`
    /// reads the range once). A range that does not follow (the head, a previous instance)
    /// never follows whatever this says.
    pub follow: bool,
    /// Wrap long lines (variable row heights) instead of letting them run off the edge.
    pub wrap: bool,
    /// Show each line's server timestamp before its text.
    pub timestamps: bool,
}

impl Default for ViewOptions {
    /// The tail of the default container, following.
    fn default() -> Self {
        Self {
            range: LogRange::default(),
            container: None,
            previous: false,
            tail_lines: None,
            follow: true,
            wrap: false,
            timestamps: false,
        }
    }
}

impl ViewOptions {
    /// The port's request for these options. Timestamps are always asked of the server: the
    /// viewer shows them on demand without a new read, and the multi-pod merge orders by them.
    /// A previous instance has stopped, so its read never follows.
    pub fn log_options(&self) -> LogOptions {
        let follow = self.follow && self.range.follows() && !self.previous;
        let mut options = LogOptions {
            container: self.container.clone(),
            follow,
            timestamps: true,
            previous: self.previous,
            ..LogOptions::default()
        };
        match self.range {
            LogRange::Tail => {
                options.tail_lines = Some(self.tail_lines.map_or(TAIL_LINES, i64::from));
            }
            LogRange::Head => options.limit_bytes = Some(HEAD_LIMIT_BYTES),
            since => options.since = since.since_seconds().map(LogSince::Seconds),
        }
        options
    }
}

/// What `pod::ViewLogs` asks of the log view of its pod: the container, the previous instance,
/// whether to follow and how long a tail to read. [`OpenLogs::default`] is the "View Logs" row
/// action's: the default container's tail, following.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenLogs {
    /// The container to read; `None` keeps the view's (a new view's: the pod's default).
    pub container: Option<String>,
    /// Read the previous (terminated) instance.
    pub previous: bool,
    /// Keep reading new lines.
    pub follow: bool,
    /// Read the last N lines (the tail range); `None` keeps the view's range.
    pub tail_lines: Option<u32>,
}

impl Default for OpenLogs {
    fn default() -> Self {
        Self {
            container: None,
            previous: false,
            follow: true,
            tail_lines: None,
        }
    }
}

impl OpenLogs {
    /// Writes what is asked into `options`.
    pub fn apply(&self, options: &mut ViewOptions) {
        if let Some(container) = &self.container {
            options.container = Some(container.clone());
        }
        options.previous = self.previous;
        options.follow = self.follow;
        if let Some(lines) = self.tail_lines {
            options.range = LogRange::Tail;
            options.tail_lines = Some(lines);
        }
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

    #[test]
    fn view_logs_asks_for_its_tail_and_whether_to_follow() {
        let mut options = ViewOptions {
            range: LogRange::Last5m,
            container: Some("app".into()),
            ..ViewOptions::default()
        };
        OpenLogs {
            tail_lines: Some(50),
            follow: false,
            ..OpenLogs::default()
        }
        .apply(&mut options);
        assert_eq!(options.container.as_deref(), Some("app"), "kept");
        let request = options.log_options();
        assert_eq!(request.tail_lines, Some(50));
        assert!(!request.follow, "read once");
        assert_eq!(request.since, None);
    }
}
