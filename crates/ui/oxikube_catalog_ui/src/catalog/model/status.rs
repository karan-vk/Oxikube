//! The status badge of a catalog row.

use oxikube_domain::session::ClusterSessionState;

/// How a badge is coloured. The view maps it onto theme tokens; the model stays free of GPUI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// Nothing is happening (`Disconnected`).
    Muted,
    /// Something is in flight (`Connecting`).
    Info,
    /// Connected and healthy.
    Success,
    /// Needs the user (`AuthRequired`) or is limping (`Degraded`).
    Warning,
    /// Failed, or the kubeconfig entry is invalid.
    Error,
}

/// A status badge: a short label, a tone, and the reason where there is one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Badge {
    /// The text on the badge.
    pub label: &'static str,
    /// Its colour.
    pub tone: Tone,
    /// Why, for `Error`, `AuthRequired` and `Invalid` (already free of secrets: adapters
    /// redact before building a reason).
    pub detail: Option<String>,
}

impl Badge {
    /// The badge of a context in `state`. A context the kubeconfig does not define properly
    /// (`problem`) is `Invalid` whatever the session says: it cannot connect as written.
    pub fn of(problem: Option<&str>, state: &ClusterSessionState) -> Self {
        if let Some(problem) = problem {
            return Self {
                label: "Invalid",
                tone: Tone::Error,
                detail: Some(problem.to_owned()),
            };
        }
        let (label, tone) = match state {
            ClusterSessionState::Disconnected => ("Disconnected", Tone::Muted),
            ClusterSessionState::Connecting => ("Connecting", Tone::Info),
            ClusterSessionState::AuthRequired { .. } => ("Auth required", Tone::Warning),
            ClusterSessionState::Ready => ("Connected", Tone::Success),
            ClusterSessionState::Degraded => ("Degraded", Tone::Warning),
            ClusterSessionState::Error { .. } => ("Error", Tone::Error),
        };
        Self {
            label,
            tone,
            detail: state.reason().map(str::to_owned),
        }
    }
}
