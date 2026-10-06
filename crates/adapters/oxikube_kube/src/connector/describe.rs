//! How a connection gets its `DescribePort`.
//!
//! The renderers live in `oxikube_describe`, an adapter of its own that this crate must not
//! depend on, so the wiring (the binary) hands the connector a factory. The factory gets what a
//! renderer needs of the connection: the pooled client and discovery, and the names of the
//! cluster and context.

use std::sync::Arc;

use async_trait::async_trait;
use kube::Client;
use oxikube_domain::ids::{ClusterId, ContextName, ResourceRef};
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{DescribeOutput, DescribePort, DiscoveryPort};

/// What a describe factory is given: one connection's client and discovery.
#[derive(Clone)]
pub struct DescribeConnection {
    /// The connection's client (shared with the other ports).
    pub client: Client,
    /// The connection's discovery.
    pub discovery: Arc<dyn DiscoveryPort>,
    /// The catalog entry connected.
    pub cluster: ClusterId,
    /// Its context name in the kubeconfig.
    pub context: ContextName,
}

impl std::fmt::Debug for DescribeConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DescribeConnection")
            .field("cluster", &self.cluster)
            .field("context", &self.context)
            .finish_non_exhaustive()
    }
}

/// Builds the `DescribePort` of one connection.
pub type DescribeFactory = dyn Fn(DescribeConnection) -> Arc<dyn DescribePort> + Send + Sync;

/// The describe port of a connector nobody gave a factory: it cannot describe anything.
pub(super) struct NoDescribe;

#[async_trait]
impl DescribePort for NoDescribe {
    async fn describe(&self, _: &ResourceRef) -> OxiResult<DescribeOutput> {
        Err(OxiError::unsupported(
            "describe is not wired into this connector",
        ))
    }
}
