//! [`MetricsKind`] (what is polled) and [`MetricsState`] (how the last poll of it went).

use std::sync::Arc;

use jiff::Timestamp;
use oxikube_domain::metrics::MissingReason;

/// What a consumer looks at, and so what is polled: one `metrics.k8s.io` call each.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MetricsKind {
    /// `NodeMetrics`: the Nodes table, the overview.
    Nodes,
    /// `PodMetrics` in the session's namespaces, with each pod's containers: the Pods table, a
    /// pod's containers.
    Pods,
}

impl MetricsKind {
    /// Both kinds, in index order.
    pub const ALL: [MetricsKind; 2] = [MetricsKind::Nodes, MetricsKind::Pods];

    /// The kind's slot in per-kind arrays.
    pub(crate) const fn index(self) -> usize {
        match self {
            MetricsKind::Nodes => 0,
            MetricsKind::Pods => 1,
        }
    }
}

/// How the last poll of one kind of one cluster went: what a table's badge shows.
///
/// Absence is a state, never a zero (ADR 0011): every state but [`Idle`](Self::Idle),
/// [`Loading`](Self::Loading) and [`Available`](Self::Available) has an
/// [`unavailable_text`](Self::unavailable_text).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum MetricsState {
    /// Not polled: nobody has looked at this kind of this cluster yet, or it disconnected.
    #[default]
    Idle,
    /// Asked for; the first answer has not come back.
    Loading,
    /// Samples, as of `at` (the time of the poll).
    Available {
        /// When the poll answered.
        at: Timestamp,
    },
    /// The metrics API cannot answer for this cluster: not installed, not answering, or not
    /// allowed. Its samples are dropped.
    Unavailable(MissingReason),
    /// The call itself failed (network, timeout, expired credentials). Retried with a growing
    /// interval; the last samples stay until a poll answers.
    Failed {
        /// What went wrong, as the error said it (no credentials: the adapters redact).
        message: Arc<str>,
    },
}

impl MetricsState {
    /// Whether samples answered the last poll.
    pub fn is_available(&self) -> bool {
        matches!(self, MetricsState::Available { .. })
    }

    /// `metrics unavailable: <reason>`, or `None` while metrics are (or are about to be) there.
    pub fn unavailable_text(&self) -> Option<String> {
        let reason = match self {
            MetricsState::Idle | MetricsState::Loading | MetricsState::Available { .. } => {
                return None;
            }
            MetricsState::Unavailable(reason) => reason_text(*reason),
            MetricsState::Failed { message } => message,
        };
        Some(format!("metrics unavailable: {reason}"))
    }
}

/// The words for a [`MissingReason`].
fn reason_text(reason: MissingReason) -> &'static str {
    match reason {
        MissingReason::NotInstalled => "metrics-server is not installed (no metrics.k8s.io API)",
        MissingReason::ApiUnavailable => {
            "the metrics.k8s.io API is not answering (metrics-server starting or failing)"
        }
        MissingReason::NotYetAvailable => "no samples yet",
        MissingReason::Forbidden => "no permission to read metrics.k8s.io",
        MissingReason::Other => "the metrics API answered with data that could not be read",
    }
}
