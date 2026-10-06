//! [`LogState`]: where a session is in its life, as the viewer words it.

use oxikube_domain::redact::redact;
use oxikube_domain::{ErrorKind, OxiError};

/// Why a session stopped without an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndReason {
    /// A read that does not follow (`follow` off, `previous`) reached the end of the log.
    Completed,
    /// A followed stream ended: the container stopped, the pod went away, or the connection was
    /// closed. Whether to follow a replacement is the reconnect policy (E08-S07).
    StreamClosed,
    /// The session was dropped or cancelled by its owner.
    Cancelled,
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

/// The state machine of a session: `Connecting`, then `Streaming`, then `Ended` or `Failed`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogState {
    /// The stream is being opened.
    Connecting,
    /// The stream is open; lines arrive as the server sends them.
    Streaming,
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
