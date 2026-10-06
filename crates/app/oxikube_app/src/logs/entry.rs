//! [`LogEntry`]: one buffered line, in the compact form the ring keeps.

use std::sync::Arc;

use jiff::Timestamp;
use oxikube_domain::log::LogLine;

/// One line in a [`LogBuffer`](super::LogBuffer).
///
/// Compact on purpose (a session holds up to `logs.buffer_lines` of them): the text is one
/// `Arc<str>` with no spare capacity, and the pod and container names are the adapter's shared
/// `Arc<str>`s, so cloning an entry for the visible rows copies nothing but three counters.
/// The parsed JSON of [`LogLine::parsed`] is not kept; the JSON mode (E08-S05) parses the lines it
/// shows from [`text`](Self::text).
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
}

impl LogEntry {
    /// The entry for `line`; [`seq`](Self::seq) is assigned when the buffer takes it.
    pub fn new(line: LogLine) -> Self {
        Self {
            seq: 0,
            ts: line.ts,
            pod: line.pod,
            container: line.container,
            text: Arc::from(line.text),
            truncated: line.truncated,
        }
    }
}
