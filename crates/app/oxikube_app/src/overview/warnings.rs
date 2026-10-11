//! [`RecentWarnings`]: the hook the overview's warnings panel reads, and its rows.

use std::sync::Arc;

use jiff::Timestamp;
use oxikube_domain::ids::ClusterId;

/// One warning in the overview's panel: what an event said about an object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WarningRow {
    /// Namespace of the object the event is about; `None` for a cluster-scoped object.
    pub namespace: Option<Arc<str>>,
    /// The object, `Pod/web-0`.
    pub object: String,
    /// The event's reason (`BackOff`, `FailedScheduling`).
    pub reason: String,
    /// The event's message, as the cluster wrote it.
    pub message: String,
    /// How many times it happened (the event's `count`), at least 1.
    pub count: u32,
    /// When it last happened, if the event says.
    pub last_seen: Option<Timestamp>,
}

/// The newest warning events of a cluster. Implemented by the Events feed (E14), which keeps the
/// events it already watches; until it exists the overview is built with [`NoRecentWarnings`].
///
/// `latest` is called from the UI thread about once a second while the overview is open, so an
/// implementation answers from memory and never blocks or talks to the cluster. The overview
/// itself never lists `core/v1` Events.
pub trait RecentWarnings: Send + Sync {
    /// Up to `n` warnings of `cluster`, newest first.
    fn latest(&self, cluster: &ClusterId, n: usize) -> Vec<WarningRow>;
}

/// No warnings: the hook until the Events feed provides one. The panel then says so.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoRecentWarnings;

impl RecentWarnings for NoRecentWarnings {
    fn latest(&self, _: &ClusterId, _: usize) -> Vec<WarningRow> {
        Vec::new()
    }
}

/// A fixed list of warnings, the same for every cluster: for tests and previews.
#[derive(Debug, Default, Clone)]
pub struct FixedWarnings(pub Vec<WarningRow>);

impl RecentWarnings for FixedWarnings {
    fn latest(&self, _: &ClusterId, n: usize) -> Vec<WarningRow> {
        self.0.iter().take(n).cloned().collect()
    }
}
