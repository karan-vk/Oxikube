//! [`LogEntry`]: one buffered line, in the compact form the ring keeps.

use std::sync::Arc;

use jiff::Timestamp;
use oxikube_domain::log::{LogLevel, LogLine};

use super::parse::classify;

/// One line in a [`LogBuffer`](super::LogBuffer).
///
/// Compact on purpose (a session holds up to `logs.buffer_lines` of them): the text is one
/// `Arc<str>` with no spare capacity, and the pod and container names are the adapter's shared
/// `Arc<str>`s, so cloning an entry for the visible rows copies nothing but three counters.
/// The parsed JSON of [`LogLine::parsed`] is not kept: only the [`level`](Self::level) of a
/// structured line is (one byte), read once as the line is committed; the JSON mode (E08-S05)
/// parses the columns of the lines it shows from [`text`](Self::text).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    /// Position in the session's stream: 0 for the first line read, then +1 for every line, and
    /// never reused or renumbered when older lines are dropped.
    pub seq: u64,
    /// The server's timestamp (the kubelet prefix, parsed by the adapter).
    pub ts: Timestamp,
    /// Pod the line came from.
    pub pod: Arc<str>,
    /// Container the line came from.
    pub container: Arc<str>,
    /// The line without its newline (at most `MAX_LOG_LINE_BYTES`).
    pub text: Arc<str>,
    /// Whether the text was cut to the per-line cap.
    pub truncated: bool,
    /// The normalised level when the text is a structured (JSON object) line
    /// ([`LogLevel::Unknown`] when it names none); `None` for plain text. See
    /// [`parse::classify`](super::parse::classify).
    pub level: Option<LogLevel>,
}

impl LogEntry {
    /// The entry for `line`; [`seq`](Self::seq) is assigned when the buffer takes it.
    pub fn new(line: LogLine) -> Self {
        let level = classify(&line.text, line.truncated);
        Self {
            seq: 0,
            ts: line.ts,
            pod: line.pod,
            container: line.container,
            text: Arc::from(line.text),
            truncated: line.truncated,
            level,
        }
    }
}
