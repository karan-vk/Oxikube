//! [`PrometheusPort`]: the per-cluster [`PromqlPort`] on top of a [`PromqlEndpointPort`].
//!
//! It holds the endpoint each cluster's metrics come from. Who fills that in is up to the
//! caller: settings (a configured URL) and detection (E13-S05) call
//! [`set_endpoint`](PrometheusPort::set_endpoint). A cluster with no endpoint simply has no
//! Prometheus: [`is_available`](PromqlPort::is_available) is `Ok(false)` and queries fail with
//! `Unsupported`.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use jiff::Timestamp;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{
    PromEndpoint, PromqlEndpointPort, PromqlPort, PromqlSeries, SecretStorePort, TimeRange,
};
use parking_lot::RwLock;

use crate::client::PromqlClient;

/// `PromqlPort` backed by one HTTP client and a table of per-cluster endpoints.
pub struct PrometheusPort {
    client: Arc<dyn PromqlEndpointPort>,
    endpoints: RwLock<HashMap<ClusterId, Arc<PromEndpoint>>>,
}

impl PrometheusPort {
    /// A port that queries through `client`.
    pub fn new(client: Arc<dyn PromqlEndpointPort>) -> Self {
        Self {
            client,
            endpoints: RwLock::default(),
        }
    }

    /// A port on the HTTP [`PromqlClient`], reading keychain credentials from `secrets`.
    pub fn http(secrets: Arc<dyn SecretStorePort>) -> Self {
        Self::new(Arc::new(PromqlClient::with_secret_store(secrets)))
    }

    /// Sets (or replaces) the endpoint `cluster`'s queries go to.
    pub fn set_endpoint(&self, cluster: ClusterId, endpoint: PromEndpoint) {
        self.endpoints.write().insert(cluster, Arc::new(endpoint));
    }

    /// Forgets `cluster`'s endpoint: it has no Prometheus until one is set again.
    pub fn clear_endpoint(&self, cluster: &ClusterId) {
        self.endpoints.write().remove(cluster);
    }

    /// The endpoint set for `cluster`.
    pub fn endpoint(&self, cluster: &ClusterId) -> Option<Arc<PromEndpoint>> {
        self.endpoints.read().get(cluster).cloned()
    }

    fn require(&self, cluster: &ClusterId) -> OxiResult<Arc<PromEndpoint>> {
        self.endpoint(cluster).ok_or_else(|| {
            OxiError::unsupported(format!(
                "no Prometheus endpoint is set for cluster {cluster}"
            ))
        })
    }
}

#[async_trait]
impl PromqlPort for PrometheusPort {
    async fn is_available(&self, cluster: &ClusterId) -> OxiResult<bool> {
        let Some(endpoint) = self.endpoint(cluster) else {
            return Ok(false);
        };
        match self.client.ping(&endpoint).await {
            Ok(()) => Ok(true),
            // Nothing there: the normal "not installed" answer. Anything else (a refused
            // credential, a wrong path, a TLS problem) is a reason the user should see.
            Err(error) if error.is_absent() => Ok(false),
            Err(error) => Err(error.into()),
        }
    }

    async fn query_instant(
        &self,
        cluster: &ClusterId,
        query: &str,
        at: Option<Timestamp>,
    ) -> OxiResult<Vec<PromqlSeries>> {
        let endpoint = self.require(cluster)?;
        self.client
            .instant_query(&endpoint, query, at)
            .await
            .map_err(OxiError::from)
    }

    async fn query_range(
        &self,
        cluster: &ClusterId,
        query: &str,
        range: TimeRange,
    ) -> OxiResult<Vec<PromqlSeries>> {
        let endpoint = self.require(cluster)?;
        self.client
            .range_query(&endpoint, query, range)
            .await
            .map_err(OxiError::from)
    }
}

impl std::fmt::Debug for PrometheusPort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PrometheusPort")
            .field("clusters", &self.endpoints.read().len())
            .finish_non_exhaustive()
    }
}
