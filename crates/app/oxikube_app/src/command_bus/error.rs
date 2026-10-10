//! [`DispatchError`]: why a dispatch did not complete.

use oxikube_domain::audit::Initiator;
use oxikube_domain::command::CommandId;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_domain::{ErrorKind, OxiError};

use crate::guard::ConfirmationError;

/// Why [`CommandBus::dispatch`](super::CommandBus::dispatch) did not complete.
///
/// Typed so the UI can explain each case (a read-only badge, a "connect first" hint);
/// [`From<DispatchError> for OxiError`](OxiError) maps it to the shared error kinds for
/// the MCP server and toasts.
#[derive(Debug, thiserror::Error)]
pub enum DispatchError {
    /// No handler is registered for the command.
    #[error("no handler is registered for {0}")]
    UnknownCommand(CommandId),
    /// The command is privileged (it changes the safety posture) and the initiator is
    /// an agent or a plugin.
    #[error("{command} cannot be run by {initiator}")]
    NotPermitted {
        /// The command refused.
        command: CommandId,
        /// Who asked.
        initiator: Initiator,
    },
    /// [`CommandBus::dispatch_now`](super::CommandBus::dispatch_now) for a command whose
    /// handler is async: dispatch it with [`CommandBus::dispatch`](super::CommandBus::dispatch).
    #[error("{0} is not an immediate command")]
    NotImmediate(CommandId),
    /// A mutating command names no cluster and the context has no active cluster.
    #[error("{0} needs a cluster")]
    NoCluster(CommandId),
    /// The cluster has no open session.
    #[error("cluster {0} is not open")]
    NoSession(ClusterId),
    /// The cluster is in read-only mode: every mutation is refused, for every initiator.
    #[error("cluster {context} is read-only")]
    ReadOnly {
        /// The cluster.
        cluster: ClusterId,
        /// Its kubeconfig context, for the message.
        context: ContextName,
    },
    /// The cluster's session is not connected, so there is nothing to write to.
    #[error("cluster {context} is not connected")]
    NotConnected {
        /// The cluster.
        cluster: ClusterId,
        /// Its kubeconfig context, for the message.
        context: ContextName,
    },
    /// The confirmation in the context was not accepted.
    #[error(transparent)]
    Confirmation(#[from] ConfirmationError),
    /// The audit log cannot be written, so the mutation was refused before it ran.
    #[error("the audit log is unavailable, mutation refused: {0}")]
    AuditUnavailable(OxiError),
    /// The mutation ran but its audit record could not be written; it is kept and
    /// retried, and further mutations are refused until it is.
    #[error("the mutation's audit record could not be written: {0}")]
    AuditFailed(OxiError),
    /// The handler failed.
    #[error(transparent)]
    Handler(OxiError),
}

impl DispatchError {
    /// The cluster a [`ReadOnly`](Self::ReadOnly) error names.
    pub fn read_only_cluster(&self) -> Option<&ClusterId> {
        match self {
            DispatchError::ReadOnly { cluster, .. } => Some(cluster),
            _ => None,
        }
    }
}

impl From<DispatchError> for OxiError {
    /// `ReadOnly` and `NotPermitted` are `Forbidden` (the user, not the cluster, said
    /// no; `ErrorKind` has no `ReadOnly` variant, see `docs/CONTEXT.md`).
    fn from(err: DispatchError) -> Self {
        let kind = match err {
            DispatchError::Handler(inner) => return inner,
            DispatchError::UnknownCommand(_) | DispatchError::NoSession(_) => ErrorKind::NotFound,
            DispatchError::NotPermitted { .. } | DispatchError::ReadOnly { .. } => {
                ErrorKind::Forbidden
            }
            DispatchError::NoCluster(_) | DispatchError::Confirmation(_) => ErrorKind::Validation,
            DispatchError::NotImmediate(_) => ErrorKind::Unsupported,
            DispatchError::NotConnected { .. } => ErrorKind::Conflict,
            DispatchError::AuditUnavailable(_) | DispatchError::AuditFailed(_) => {
                ErrorKind::Internal
            }
        };
        OxiError::new(kind, err.to_string())
    }
}
