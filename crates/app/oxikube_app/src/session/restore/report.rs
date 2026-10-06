//! What a restore's connects did.

use oxikube_domain::ids::ClusterId;
use oxikube_domain::session::{ClusterSessionState, SessionPhase};

/// How one cluster's connect ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectOutcome {
    /// The cluster.
    pub cluster: ClusterId,
    /// The session's state when the attempt ended: `Ready`, `AuthRequired`, `Error` (a failure or
    /// the timeout), or whatever it already was when `attempted` is `false`.
    pub state: ClusterSessionState,
    /// `false` when nothing was started because the session was not disconnected any more (the
    /// user connected it first) or no longer exists.
    pub attempted: bool,
}

impl ConnectOutcome {
    /// Whether the cluster is not usable: it needs credentials or the connect failed.
    pub fn failed(&self) -> bool {
        matches!(
            self.state.phase(),
            SessionPhase::AuthRequired | SessionPhase::Error
        )
    }
}

/// The outcomes of [`SessionRestorer::connect`](super::SessionRestorer::connect), in the order
/// the attempts finished.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RestoreReport {
    /// One entry per cluster the restore tried to connect.
    pub outcomes: Vec<ConnectOutcome>,
}

impl RestoreReport {
    /// The outcome for `cluster`.
    pub fn outcome(&self, cluster: &ClusterId) -> Option<&ConnectOutcome> {
        self.outcomes.iter().find(|o| &o.cluster == cluster)
    }

    /// The clusters that did not come up.
    pub fn failures(&self) -> impl Iterator<Item = &ConnectOutcome> {
        self.outcomes.iter().filter(|o| o.failed())
    }
}
