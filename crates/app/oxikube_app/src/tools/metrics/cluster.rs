//! The cluster a metrics call is about.

use oxikube_domain::ids::ClusterId;
use oxikube_domain::{ErrorKind, OxiError, OxiResult};
use oxikube_ports::{ToolContext, ToolOutput};

use crate::session::{ClusterSession, ClusterSessionManager};

/// The session of `cluster`, or of the only connected cluster when `None`.
///
/// # Errors
///
/// `NotFound` for a cluster with no session, `Network` for one that is not connected, and a
/// validation error when none is named and several (or no) clusters are connected.
pub(crate) fn session_for(
    sessions: &ClusterSessionManager,
    cluster: Option<&ClusterId>,
) -> OxiResult<ClusterSession> {
    match cluster {
        Some(id) => {
            let session = sessions
                .get(id)
                .ok_or_else(|| OxiError::not_found(format!("no session for cluster {id}")))?;
            if session.is_connected() {
                Ok(session)
            } else {
                Err(OxiError::network(format!(
                    "cluster {} is not connected",
                    session.title()
                )))
            }
        }
        None => {
            let mut connected = sessions.sessions().into_iter().filter(|s| s.is_connected());
            match (connected.next(), connected.next()) {
                (Some(only), None) => Ok(only),
                (None, _) => Err(OxiError::network("no cluster is connected")),
                (Some(_), Some(_)) => Err(OxiError::validation(
                    "several clusters are connected: name the cluster",
                )),
            }
        }
    }
}

/// [`session_for`] for a tool call: a validation error stays an `Err` (the call is malformed),
/// anything else is a tool error the model sees.
pub(super) fn resolve(
    sessions: &ClusterSessionManager,
    ctx: &ToolContext,
) -> OxiResult<Result<ClusterSession, ToolOutput>> {
    match session_for(sessions, ctx.cluster.as_ref()) {
        Ok(session) => Ok(Ok(session)),
        Err(error) if error.kind() == ErrorKind::Validation => Err(error),
        Err(error) => Ok(Err(ToolOutput::error(format!(
            "{}: {}",
            error.kind(),
            oxikube_domain::redact::redact(error.message())
        )))),
    }
}
