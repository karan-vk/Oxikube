//! [`AggregateSession`]: a multi-pod log session and its [`AggregateView`].

use oxikube_ports::{LogPort, ResourceReader};
use std::sync::Arc;

use super::view::AggregateView;
use crate::logs::LogSession;

/// The ports an aggregate reads through: the cluster's log streams, and its objects (to resolve a
/// workload's selector and to follow its pods).
#[derive(Clone)]
pub struct AggregatePorts {
    /// Opens each container's stream.
    pub logs: Arc<dyn LogPort>,
    /// Reads the workload or Service and watches the pods its selector matches.
    pub resources: Arc<dyn ResourceReader>,
}

impl std::fmt::Debug for AggregatePorts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AggregatePorts").finish_non_exhaustive()
    }
}

/// One open multi-pod log: the merged lines (a [`LogSession`], read like any other: buffer,
/// deltas, state) and what a multi-pod viewer adds (the [`AggregateView`]). Dropping it cancels
/// the pod watch and every stream.
pub struct AggregateSession {
    session: LogSession,
    view: AggregateView,
}

impl AggregateSession {
    pub(crate) fn new(session: LogSession, view: AggregateView) -> Self {
        Self { session, view }
    }

    /// The streams, pod events and hidden sources.
    pub fn aggregate(&self) -> &AggregateView {
        &self.view
    }

    /// Splits the session into the part that owns the streams and the view of the rest. The
    /// [`LogSession`] must be kept for as long as the streams should run.
    pub fn into_parts(self) -> (LogSession, AggregateView) {
        (self.session, self.view)
    }
}

impl std::ops::Deref for AggregateSession {
    type Target = LogSession;

    fn deref(&self) -> &LogSession {
        &self.session
    }
}
