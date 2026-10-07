//! [`LogState`]: where a session is in its life, as the viewer words it.

use oxikube_domain::redact::redact;
use oxikube_domain::{ErrorKind, OxiError};

/// Why a session stopped without an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndReason {
    /// A read that does not follow (`follow` off, `previous`) reached the end of the log.
    Completed,
    /// A followed stream ended and why could not be told (the session cannot read its pod).
    StreamClosed,
    /// The session was dropped or cancelled by its owner.
    Cancelled,
    /// The pod ran to its end (`Succeeded` or `Failed`): nothing more will be written.
    PodFinished,
    /// The followed container exited and will not run again while its pod runs on (a completed
    /// init container, a container that finished next to a sidecar): its log is complete.
    ContainerFinished,
    /// The pod was deleted (or is terminating, or was recreated under its name) and a controller
    /// owns it: a new pod takes over, and the viewer offers to follow it
    /// ([`find_replacement`](super::find_replacement)).
    PodReplaced,
    /// The pod was deleted and nothing owns it: there is no replacement to follow.
    PodDeleted,
}

/// Why a session failed: the error of opening the stream (pod not found, `pods/log` forbidden, a
/// container that has not started) or the one that broke it. Redacted: it may be shown and
/// logged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogFailure {
    /// The error's classification (`NotFound`, `Forbidden`, `Validation`, `Network`, ...).
    pub kind: ErrorKind,
    /// The error message, scrubbed of secrets.
    pub message: String,
    /// Whether trying again may succeed (a connection failure, a timeout).
    pub retryable: bool,
}

impl From<&OxiError> for LogFailure {
    fn from(error: &OxiError) -> Self {
        Self {
            kind: error.kind(),
            message: redact(error.message()).into_owned(),
            retryable: error.is_retryable(),
        }
    }
}

/// The state machine of a session: `Connecting`, then `Streaming`, then `Ended` or `Failed`. A
/// stream that breaks while its pod runs goes `Reconnecting` and back to `Streaming` (E08-S07).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogState {
    /// The stream is being opened (or its container is waiting to start).
    Connecting,
    /// The stream is open; lines arrive as the server sends them.
    Streaming,
    /// The stream broke while its pod runs: reconnect `attempt` of `max` is due after a pause.
    /// Lines already read stay; the replayed overlap is dropped when the stream is back.
    Reconnecting {
        /// Which attempt (1 for the first).
        attempt: u32,
        /// Attempts before the session gives up (`logs.reconnect_retries`).
        max: u32,
        /// What broke the stream, redacted.
        failure: LogFailure,
    },
    /// The stream ended without an error. Lines already read stay in the buffer.
    Ended(EndReason),
    /// The stream could not be opened, or broke. Lines already read stay in the buffer.
    Failed(LogFailure),
}

impl LogState {
    /// Whether nothing more will arrive (`Ended` or `Failed`).
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Ended(_) | Self::Failed(_))
    }
}
