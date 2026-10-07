//! [`LogClusters`]: which cluster's log and object ports an excerpt reads through.

use oxikube_domain::ids::ClusterId;
use oxikube_domain::{OxiError, OxiResult};

use super::super::AggregatePorts;
use crate::session::ClusterSessionManager;

/// The read half of one connected cluster, as an excerpt needs it.
#[derive(Clone, Debug)]
pub struct LogCluster {
    /// The cluster.
    pub id: ClusterId,
    /// How the cluster is named to people (display name, else context name).
    pub title: String,
    /// The log streams and the object reader.
    pub ports: AggregatePorts,
}

/// Resolves the cluster an agent's request is about. Implemented by [`ClusterSessionManager`];
/// tests pass a fixed cluster over the fakes.
pub trait LogClusters: Send + Sync {
    /// The ports of `cluster`, or of the only connected cluster when `None`.
    ///
    /// # Errors
    ///
    /// [`NotFound`](oxikube_domain::ErrorKind::NotFound) for a cluster with no session,
    /// [`Network`](oxikube_domain::ErrorKind::Network) for one that is not connected, and a
    /// validation error when none is named and several (or no) clusters are connected.
    fn cluster(&self, cluster: Option<&ClusterId>) -> OxiResult<LogCluster>;
}

impl LogClusters for ClusterSessionManager {
    fn cluster(&self, cluster: Option<&ClusterId>) -> OxiResult<LogCluster> {
        let session = match cluster {
            Some(id) => self
                .get(id)
                .ok_or_else(|| OxiError::not_found(format!("no session for cluster {id}")))?,
            None => {
                let mut connected = self.sessions().into_iter().filter(|s| s.is_connected());
                match (connected.next(), connected.next()) {
                    (Some(only), None) => only,
                    (None, _) => return Err(OxiError::network("no cluster is connected")),
                    (Some(_), Some(_)) => {
                        return Err(OxiError::validation(
                            "several clusters are connected: name the cluster",
                        ));
                    }
                }
            }
        };
        let (Some(logs), Some(resources)) = (session.logs(), session.resources()) else {
            return Err(OxiError::network(format!(
                "cluster {} is not connected",
                session.title()
            )));
        };
        Ok(LogCluster {
            id: session.id().clone(),
            title: session.title().to_owned(),
            ports: AggregatePorts { logs, resources },
        })
    }
}
