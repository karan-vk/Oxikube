//! [`EventsState`]: how a cluster's events feed is doing, as a view says it.

use oxikube_domain::{ErrorKind, OxiError};

/// The state of a cluster's events feed in the [`EventService`](super::EventService).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum EventsState {
    /// No feed runs: nobody subscribes, or the cluster is not connected. The buffer is empty.
    #[default]
    Idle,
    /// The feed is opening or listing; the events shown may be missing or stale.
    Warming,
    /// Listed and watching; the events are current.
    Ready,
    /// A failure being retried; the events held stay.
    Retrying {
        /// What failed (already redacted by the adapter).
        message: String,
    },
    /// A failure that is not retried until [`EventService::retry`](super::EventService::retry),
    /// a reconnect or a new namespace selection (the user may not read events anywhere, the
    /// cluster serves no events API).
    Failed {
        /// The error kind.
        kind: ErrorKind,
        /// What failed (already redacted by the adapter).
        message: String,
    },
}

impl EventsState {
    pub(crate) fn retrying(error: &OxiError) -> Self {
        Self::Retrying {
            message: error.message().to_owned(),
        }
    }

    pub(crate) fn failed(error: &OxiError) -> Self {
        Self::Failed {
            kind: error.kind(),
            message: error.message().to_owned(),
        }
    }

    /// Whether the feed stopped for good.
    pub fn is_failed(&self) -> bool {
        matches!(self, Self::Failed { .. })
    }
}
