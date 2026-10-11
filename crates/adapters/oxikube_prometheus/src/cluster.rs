//! [`PrometheusPort`]: the per-cluster [`PromqlPort`] on top of a [`PromqlEndpointPort`].
//!
//! It holds the endpoint each cluster's metrics come from. Who fills that in is up to the
//! caller: settings (a configured URL or service) and detection (E13-S05) call
//! [`set_endpoint`](PrometheusPort::set_endpoint). A cluster with no endpoint simply has no
//! Prometheus: [`is_available`](PromqlPort::is_available) is `Ok(false)` and queries fail with
//! `Unsupported`.
//!
//! An endpoint on a Service (what detection finds) is queried through the cluster's service
//! proxy, which the port looks up per query in its [`ServiceProxies`], so a reconnect that
//! replaces the connection needs no update here. While the cluster is not connected such a query
//! fails with an unreachable error that says so.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use jiff::Timestamp;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{
    PromEndpoint, PromqlEndpointPort, PromqlError, PromqlPort, PromqlSeries, SecretStorePort,
    ServiceProxyPort, TimeRange,
};
use parking_lot::RwLock;

use crate::client::PromqlClient;
use crate::proxy::ProxyPromqlClient;

/// Where the port finds a connected cluster's service proxy.
pub trait ServiceProxies: Send + Sync {
    /// The service proxy of `cluster`'s live connection, `None` while it is not connected.
    fn service_proxy(&self, cluster: &ClusterId) -> Option<Arc<dyn ServiceProxyPort>>;
}

/// `PromqlPort` backed by one HTTP client and a table of per-cluster endpoints.
pub struct PrometheusPort {
    client: Arc<dyn PromqlEndpointPort>,
    proxies: Option<Arc<dyn ServiceProxies>>,
    endpoints: RwLock<HashMap<ClusterId, Arc<PromEndpoint>>>,
}

impl PrometheusPort {
    /// A port that queries through `client`.
    pub fn new(client: Arc<dyn PromqlEndpointPort>) -> Self {
        Self {
            client,
            proxies: None,
            endpoints: RwLock::default(),
        }
    }

    /// The same port, reaching Service endpoints through the clusters' service proxies.
    #[must_use]
    pub fn with_proxies(mut self, proxies: Arc<dyn ServiceProxies>) -> Self {
        self.proxies = Some(proxies);
        self
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

    /// The client that can send to `endpoint`: the direct one for a URL, the cluster's service
    /// proxy for a Service.
    fn client_for(
        &self,
        cluster: &ClusterId,
        endpoint: &PromEndpoint,
    ) -> Result<Arc<dyn PromqlEndpointPort>, PromqlError> {
        if endpoint.service.is_none() {
            return Ok(self.client.clone());
        }
        let proxy = self
            .proxies
            .as_ref()
            .and_then(|proxies| proxies.service_proxy(cluster))
            .ok_or_else(|| {
                PromqlError::Unreachable(format!(
                    "cluster {cluster} is not connected, so {} cannot be reached",
                    endpoint.location()
                ))
            })?;
        Ok(Arc::new(ProxyPromqlClient::new(proxy, self.client.clone())))
    }
}

#[async_trait]
impl PromqlPort for PrometheusPort {
    async fn is_available(&self, cluster: &ClusterId) -> OxiResult<bool> {
        let Some(endpoint) = self.endpoint(cluster) else {
            return Ok(false);
        };
        let outcome = match self.client_for(cluster, &endpoint) {
            Ok(client) => client.ping(&endpoint).await,
            Err(error) => Err(error),
        };
        match outcome {
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
        let client = self.client_for(cluster, &endpoint)?;
        client
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
        let client = self.client_for(cluster, &endpoint)?;
        client
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
