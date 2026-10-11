//! [`ProxyPromqlClient`]: queries a Service through the API server's service proxy.
//!
//! An endpoint with a [`service`](PromEndpoint::service) is not a URL: the request is sent to
//! the cluster's `ServiceProxyPort`, which carries the connection's credentials, so the endpoint's
//! own auth and TLS settings do not apply. An endpoint with a URL goes to the direct client. One
//! of these exists per cluster; it is cheap to build.

use std::sync::Arc;

use async_trait::async_trait;
use jiff::Timestamp;
use oxikube_ports::{
    PromEndpoint, PromqlEndpointPort, PromqlError, PromqlSeries, ProxyMethod, ProxyRequest,
    ServiceProxyPort, ServiceRef, TimeRange,
};

use crate::decode::decode;
use crate::errors;
use crate::url::{MAX_QUERY_IN_URL, Route, api_path, unix_seconds};

/// A [`PromqlEndpointPort`] that sends service endpoints through a cluster's service proxy and
/// everything else through `direct`.
#[derive(Clone)]
pub struct ProxyPromqlClient {
    proxy: Arc<dyn ServiceProxyPort>,
    direct: Arc<dyn PromqlEndpointPort>,
}

impl ProxyPromqlClient {
    /// A client for one cluster: `proxy` is its service proxy, `direct` talks to URLs.
    pub fn new(proxy: Arc<dyn ServiceProxyPort>, direct: Arc<dyn PromqlEndpointPort>) -> Self {
        Self { proxy, direct }
    }

    async fn run(
        &self,
        endpoint: &PromEndpoint,
        service: &ServiceRef,
        route: Route,
        query: &str,
        params: Vec<(&'static str, String)>,
    ) -> Result<Vec<PromqlSeries>, PromqlError> {
        if query.trim().is_empty() {
            return Err(PromqlError::BadQuery {
                message: "the query is empty".into(),
            });
        }
        let mut fields = vec![("query".to_owned(), query.to_owned())];
        fields.extend(params.into_iter().map(|(k, v)| (k.to_owned(), v)));
        let request = ProxyRequest {
            method: if query.len() > MAX_QUERY_IN_URL {
                ProxyMethod::PostForm
            } else {
                ProxyMethod::Get
            },
            path: api_path(endpoint, route),
            params: fields,
            timeout: endpoint.timeout,
            max_response_bytes: endpoint.max_response_bytes,
        };
        tracing::debug!(
            target: "oxikube_prometheus",
            service = %service,
            path = %request.path,
            query = %errors::clean(query),
            "promql request through the service proxy"
        );
        let answer = self
            .proxy
            .send(service, request)
            .await
            .map_err(|e| errors::from_proxy(&e, endpoint.timeout))?;
        if answer.truncated {
            return Err(PromqlError::BadResponse(format!(
                "the response is larger than the {} byte limit; narrow the query",
                endpoint.max_response_bytes
            )));
        }
        if let Some(message) = &answer.api_server_message {
            return Err(errors::from_api_server(answer.status, message));
        }
        decode(answer.status, &answer.body)
    }
}

#[async_trait]
impl PromqlEndpointPort for ProxyPromqlClient {
    async fn instant_query(
        &self,
        endpoint: &PromEndpoint,
        query: &str,
        at: Option<Timestamp>,
    ) -> Result<Vec<PromqlSeries>, PromqlError> {
        let Some(service) = &endpoint.service else {
            return self.direct.instant_query(endpoint, query, at).await;
        };
        let params = at
            .map(|at| vec![("time", unix_seconds(at))])
            .unwrap_or_default();
        self.run(endpoint, service, Route::Instant, query, params)
            .await
    }

    async fn range_query(
        &self,
        endpoint: &PromEndpoint,
        query: &str,
        range: TimeRange,
    ) -> Result<Vec<PromqlSeries>, PromqlError> {
        let Some(service) = &endpoint.service else {
            return self.direct.range_query(endpoint, query, range).await;
        };
        if range.step.is_zero() {
            return Err(PromqlError::BadQuery {
                message: "the range step must be greater than zero".into(),
            });
        }
        if range.end < range.start {
            return Err(PromqlError::BadQuery {
                message: "the range ends before it starts".into(),
            });
        }
        let params = vec![
            ("start", unix_seconds(range.start)),
            ("end", unix_seconds(range.end)),
            ("step", range.step.as_secs_f64().to_string()),
        ];
        self.run(endpoint, service, Route::Range, query, params)
            .await
    }
}

impl std::fmt::Debug for ProxyPromqlClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProxyPromqlClient").finish_non_exhaustive()
    }
}
