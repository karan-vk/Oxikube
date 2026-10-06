//! [`ClusterCommands`]: the handler of the cluster commands the catalog dispatches.

use oxikube_domain::command::Command;
use oxikube_domain::session::{ClusterSessionState, SessionPhase};
use oxikube_domain::{OxiError, OxiResult};

use super::service::ClusterCatalog;
use crate::session::ClusterSessionManager;

/// What a cluster command did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClusterCommandOutcome {
    /// `cluster::Connect` or `cluster::Reconnect` ran: the state the attempt ended in (`Ready`,
    /// `AuthRequired`, `Error`, or `Disconnected` when it was cancelled).
    Connected(ClusterSessionState),
    /// `cluster::CancelConnect` ran: whether an attempt was in flight and is now cancelled.
    Cancelled(bool),
    /// `cluster::Disconnect` ran.
    Disconnected,
    /// `cluster::ToggleFavourite` ran: the favourite flag now.
    Favourite(bool),
}

/// Runs `cluster::Connect`, `cluster::Reconnect`, `cluster::CancelConnect`,
/// `cluster::Disconnect` and `cluster::ToggleFavourite`.
///
/// The `CommandBus` (E06-S02) registers [`handle`](Self::handle) for those ids. None of them
/// mutates a cluster, so none goes through `MutationGuard`.
#[derive(Debug, Clone)]
pub struct ClusterCommands {
    sessions: ClusterSessionManager,
    catalog: ClusterCatalog,
}

impl ClusterCommands {
    /// A handler over `sessions` and `catalog`.
    pub fn new(sessions: ClusterSessionManager, catalog: ClusterCatalog) -> Self {
        Self { sessions, catalog }
    }

    /// Whether [`handle`](Self::handle) runs `command`.
    pub fn handles(command: &Command) -> bool {
        matches!(
            command,
            Command::ClusterConnect { .. }
                | Command::ClusterReconnect { .. }
                | Command::ClusterCancelConnect { .. }
                | Command::ClusterDisconnect { .. }
                | Command::ClusterToggleFavourite { .. }
        )
    }

    /// Runs `command`.
    ///
    /// Connecting stamps the cluster as used first (best effort: a state db that fails is
    /// logged and the connect still runs), then waits for the attempt to end. Run it off the
    /// UI thread (`oxikube_runtime::spawn_kube`); dropping the future cancels the attempt.
    /// Disconnecting a cluster that was never opened is a no-op.
    ///
    /// # Errors
    ///
    /// `Validation` for a command this handler does not own; `NotFound` for a connect of a
    /// cluster that is not in the catalog; the state port's error for a favourite change.
    /// Connection failures are an `Ok(Connected(Error { .. }))`, not an error.
    pub async fn handle(&self, command: &Command) -> OxiResult<ClusterCommandOutcome> {
        match command {
            Command::ClusterConnect { cluster } => {
                if let Err(error) = self.catalog.mark_used(cluster).await {
                    tracing::warn!(%error, %cluster, "could not record the cluster as used");
                }
                self.sessions
                    .connect(cluster)
                    .await
                    .map(ClusterCommandOutcome::Connected)
            }
            Command::ClusterReconnect { cluster } => {
                if let Err(error) = self.catalog.mark_used(cluster).await {
                    tracing::warn!(%error, %cluster, "could not record the cluster as used");
                }
                self.sessions
                    .reconnect(cluster)
                    .await
                    .map(ClusterCommandOutcome::Connected)
            }
            Command::ClusterCancelConnect { cluster } => {
                // Only an attempt in flight is cancelled: when it ended a moment ago (the user
                // clicked as the cluster answered) the session keeps the state it reached.
                let connecting = self
                    .sessions
                    .get(cluster)
                    .is_some_and(|session| session.phase() == SessionPhase::Connecting);
                if connecting {
                    self.sessions.disconnect(cluster)?;
                }
                Ok(ClusterCommandOutcome::Cancelled(connecting))
            }
            Command::ClusterDisconnect { cluster } => {
                if self.sessions.get(cluster).is_some() {
                    self.sessions.disconnect(cluster)?;
                }
                Ok(ClusterCommandOutcome::Disconnected)
            }
            Command::ClusterToggleFavourite { cluster, favourite } => self
                .catalog
                .set_favourite(cluster, *favourite)
                .await
                .map(ClusterCommandOutcome::Favourite),
            other => Err(OxiError::validation(format!(
                "{} is not a cluster catalog command",
                other.id()
            ))),
        }
    }
}
