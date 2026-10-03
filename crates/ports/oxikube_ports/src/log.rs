//! Container logs: [`LogPort`].
//!
//! Mirrors kube's `Api<Pod>::log_stream(name, &LogParams)` (and `logs`, which
//! is the same call with `follow: false` collected). The adapter splits the
//! byte stream into domain [`LogLine`]s, parsing the RFC 3339 prefix when
//! `timestamps` is on. Reconnecting a followed stream with overlap and dedup is
//! adapter behaviour (E04-S08, kdash `stream.rs`), not a port concern; fan-in
//! across containers or pods is `oxikube_app::LogService`.

use std::pin::Pin;

use async_trait::async_trait;
use futures::Stream;
use jiff::Timestamp;
use oxikube_domain::OxiResult;
use oxikube_domain::log::LogLine;

/// Where a log read starts in time. kube's `LogParams` has
/// `since_seconds` and `since_time`, of which the server accepts at most one;
/// this enum makes the invalid combination unrepresentable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogSince {
    /// Lines newer than this many seconds (`sinceSeconds`).
    Seconds(i64),
    /// Lines at or after this instant (`sinceTime`).
    Time(Timestamp),
}

/// Options for [`LogPort::stream_logs`]. Mirrors kube `LogParams` (minus
/// `pretty`, which has no effect on logs).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LogOptions {
    /// Container to read; `None` means the pod's only (or default) container.
    pub container: Option<String>,
    /// Keep the stream open and yield new lines as they are written.
    pub follow: bool,
    /// Stop after this many bytes.
    pub limit_bytes: Option<i64>,
    /// Read the previous (terminated) instance of the container.
    pub previous: bool,
    /// Start point in time.
    pub since: Option<LogSince>,
    /// Start this many lines from the end.
    pub tail_lines: Option<i64>,
    /// Ask the server to prefix each line with its timestamp, which the
    /// adapter moves into [`LogLine::ts`].
    pub timestamps: bool,
}

impl LogOptions {
    /// Options for following a container's log: `follow` and `timestamps` on.
    pub fn follow() -> Self {
        Self {
            follow: true,
            timestamps: true,
            ..Self::default()
        }
    }

    /// Sets the container.
    #[must_use]
    pub fn container(mut self, container: impl Into<String>) -> Self {
        self.container = Some(container.into());
        self
    }

    /// Starts this many lines from the end.
    #[must_use]
    pub fn tail_lines(mut self, lines: i64) -> Self {
        self.tail_lines = Some(lines);
        self
    }

    /// Sets the start point in time.
    #[must_use]
    pub fn since(mut self, since: LogSince) -> Self {
        self.since = Some(since);
        self
    }

    /// Stops after `bytes` bytes.
    #[must_use]
    pub fn limit_bytes(mut self, bytes: i64) -> Self {
        self.limit_bytes = Some(bytes);
        self
    }

    /// Reads the previous container instance.
    #[must_use]
    pub fn previous(mut self) -> Self {
        self.previous = true;
        self
    }
}

/// A stream of log lines in the order the server sent them. Ends when the log
/// ends (or, when following, when the container stops). Dropping it closes the
/// connection.
pub type LogStream = Pin<Box<dyn Stream<Item = OxiResult<LogLine>> + Send>>;

/// Reads container logs. Read-only; the lines may contain secrets, so they go
/// through redaction before any persistence (non-negotiable 5).
#[async_trait]
pub trait LogPort: Send + Sync {
    /// Streams the log of one container of `pod`.
    async fn stream_logs(
        &self,
        namespace: &str,
        pod: &str,
        options: &LogOptions,
    ) -> OxiResult<LogStream>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_options_builders_cover_every_field() {
        let since = Timestamp::UNIX_EPOCH;
        let opts = LogOptions::follow()
            .container("app")
            .tail_lines(100)
            .since(LogSince::Time(since))
            .limit_bytes(1 << 20)
            .previous();
        assert_eq!(
            opts,
            LogOptions {
                container: Some("app".into()),
                follow: true,
                limit_bytes: Some(1 << 20),
                previous: true,
                since: Some(LogSince::Time(since)),
                tail_lines: Some(100),
                timestamps: true,
            }
        );
        let secs = LogOptions::default().since(LogSince::Seconds(300));
        assert_eq!(secs.since, Some(LogSince::Seconds(300)));
        assert!(!secs.follow && !secs.timestamps);
    }
}
