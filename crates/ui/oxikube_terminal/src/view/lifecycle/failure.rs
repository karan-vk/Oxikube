//! [`Failure`]: why a terminal could not start or lost its connection, as the words the banner
//! shows (the error taxonomy of E09-S12).
//!
//! A pod terminal fails in a handful of distinct ways, and each deserves its own sentence: an
//! expired login is fixed differently from a missing permission, a deleted pod or a stopped
//! container. The kind of the adapter's [`OxiError`] picks the sentence; the adapter's own message
//! (already redacted by the adapter, and passed through `redact` again here) is kept as the detail,
//! so the banner never says a bare "stream closed EOF". Nothing the process printed is ever part
//! of a failure.

use oxikube_domain::redact::redact;
use oxikube_domain::{ErrorKind, OxiError};

/// What went wrong, in the terms of the banner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    /// The cluster rejected the credentials (they expired): refresh the login.
    AuthExpired,
    /// RBAC denies `pods/exec`.
    Forbidden,
    /// The pod or the container does not exist (any more).
    PodGone,
    /// The container is not running: it terminated, or the pod is terminating or not started.
    ContainerStopped,
    /// The connection to the cluster dropped or timed out.
    ConnectionLost,
    /// The cluster cannot open this kind of session.
    Unsupported,
    /// The cluster refused the request as malformed (several containers and none chosen).
    Rejected,
    /// The stream ended without an exit status: nothing says why.
    StreamClosed,
    /// The command could not run in the container (executable missing, runtime refused); the
    /// server's message is the detail.
    CommandFailed,
    /// A local shell could not be started (no such program, no PTY).
    LocalStart,
    /// Anything else: a bug or a local limit.
    Unexpected,
}

/// A [`FailureKind`] with the adapter's detail. See the [module docs](self).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    kind: FailureKind,
    detail: Option<String>,
}

impl Failure {
    /// A failure of `kind` without a detail.
    pub fn new(kind: FailureKind) -> Self {
        Self { kind, detail: None }
    }

    /// Reads a remote session's error: the taxonomy of the module docs. `detail` is the error's
    /// message, redacted.
    pub fn from_error(error: &OxiError) -> Self {
        let kind = match error.kind() {
            ErrorKind::Auth => FailureKind::AuthExpired,
            ErrorKind::Forbidden => FailureKind::Forbidden,
            ErrorKind::NotFound => FailureKind::PodGone,
            ErrorKind::Conflict => FailureKind::ContainerStopped,
            ErrorKind::Network | ErrorKind::Timeout => FailureKind::ConnectionLost,
            ErrorKind::Unsupported => FailureKind::Unsupported,
            ErrorKind::Validation => FailureKind::Rejected,
            ErrorKind::Internal | ErrorKind::BudgetExceeded => FailureKind::Unexpected,
        };
        Self::with_message(kind, error)
    }

    /// A local shell that could not start: the launcher's message is the detail.
    pub fn local_start(error: &OxiError) -> Self {
        Self::with_message(FailureKind::LocalStart, error)
    }

    /// A remote command that ended with no exit code but the server's `message`.
    pub fn command_failed(message: &str) -> Self {
        Self {
            kind: FailureKind::CommandFailed,
            detail: Some(redact(message.trim()).into_owned()),
        }
    }

    fn with_message(kind: FailureKind, error: &OxiError) -> Self {
        let message = redact(error.message().trim());
        Self {
            kind,
            detail: (!message.is_empty()).then(|| message.into_owned()),
        }
    }

    /// What went wrong.
    pub fn kind(&self) -> FailureKind {
        self.kind
    }

    /// The adapter's message, redacted.
    pub fn detail(&self) -> Option<&str> {
        self.detail.as_deref()
    }

    /// The banner's headline: one distinct line per kind.
    pub fn headline(&self) -> &'static str {
        match self.kind {
            FailureKind::AuthExpired => "Authentication expired",
            FailureKind::Forbidden => "Not allowed to open a terminal here",
            FailureKind::PodGone => "The pod or container is gone",
            FailureKind::ContainerStopped => "The container is not running",
            FailureKind::ConnectionLost => "Connection lost",
            FailureKind::Unsupported => "The cluster cannot open this terminal",
            FailureKind::Rejected => "The cluster refused the request",
            FailureKind::StreamClosed => "The session was closed",
            FailureKind::CommandFailed => "The command could not run",
            FailureKind::LocalStart => "The terminal could not start",
            FailureKind::Unexpected => "The terminal failed",
        }
    }

    /// What the user can do about it: one distinct sentence per kind.
    pub fn hint(&self) -> &'static str {
        match self.kind {
            FailureKind::AuthExpired => {
                "The cluster rejected your credentials. Refresh the login for this cluster, then reconnect."
            }
            FailureKind::Forbidden => {
                "Your role may not exec into this pod (it needs create on pods/exec)."
            }
            FailureKind::PodGone => {
                "The pod was deleted or replaced. Open a terminal in the new pod."
            }
            FailureKind::ContainerStopped => {
                "The container terminated, or the pod is terminating or has not started."
            }
            FailureKind::ConnectionLost => "The connection to the cluster dropped.",
            FailureKind::Unsupported => {
                "The cluster or a proxy in front of it does not support exec streams."
            }
            FailureKind::Rejected => {
                "Choose a container in the pod, or check the command that was run."
            }
            FailureKind::StreamClosed => "The connection ended without an exit status.",
            FailureKind::CommandFailed => "The container's runtime refused to run it.",
            FailureKind::LocalStart => "The shell could not be started on this machine.",
            FailureKind::Unexpected => "Something unexpected happened. Try again.",
        }
    }
}
