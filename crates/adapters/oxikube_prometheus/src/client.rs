//! [`PromqlClient`]: the HTTP implementation of [`PromqlEndpointPort`].

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use jiff::Timestamp;
use oxikube_ports::{
    PromEndpoint, PromqlEndpointPort, PromqlError, PromqlSeries, SecretStorePort, TimeRange,
};
use reqwest::header::{ACCEPT, CONTENT_LENGTH};
use reqwest::{Method, Response};

use crate::decode::decode;
use crate::transport::ClientPool;
use crate::url::{MAX_QUERY_IN_URL, Route, api_url, loggable, unix_seconds};
use crate::{auth, errors};

/// Talks to any Prometheus-compatible HTTP API (Prometheus, VictoriaMetrics, Mimir, Thanos).
///
/// Cheap to share: clone the `Arc` or hold one for the process. The pooled HTTP clients, one per
/// TLS configuration, are built on first use. Dropping a call's future closes its connection.
#[derive(Default)]
pub struct PromqlClient {
    pool: ClientPool,
    secrets: Option<Arc<dyn SecretStorePort>>,
}

impl PromqlClient {
    /// A client with no keychain access: only in-memory credentials resolve.
    pub fn new() -> Self {
        Self::default()
    }

    /// A client that reads [`PromSecret::Keychain`](oxikube_ports::PromSecret::Keychain)
    /// credentials from `secrets` at request time.
    pub fn with_secret_store(secrets: Arc<dyn SecretStorePort>) -> Self {
        Self {
            pool: ClientPool::default(),
            secrets: Some(secrets),
        }
    }

    async fn run(
        &self,
        endpoint: &PromEndpoint,
        route: Route,
        query: &str,
        params: Vec<(&'static str, String)>,
    ) -> Result<Vec<PromqlSeries>, PromqlError> {
        if query.trim().is_empty() {
            return Err(PromqlError::BadQuery {
                message: "the query is empty".into(),
            });
        }
        if endpoint.service.is_some() {
            return Err(PromqlError::InvalidEndpoint(
                "a service endpoint is reached through the cluster's API server; this client \
                 only speaks to URLs"
                    .into(),
            ));
        }
        let mut url = api_url(endpoint, route)?;
        let post = query.len() > MAX_QUERY_IN_URL;
        let mut form = Vec::with_capacity(params.len() + 1);
        form.push(("query", query.to_owned()));
        form.extend(params);
        if !post {
            url.query_pairs_mut()
                .extend_pairs(form.iter().map(|(k, v)| (*k, v.as_str())));
        }

        tracing::debug!(
            target: "oxikube_prometheus",
            url = %loggable(&url),
            method = if post { "POST" } else { "GET" },
            query = %errors::clean(query),
            "promql request"
        );

        let client = self.pool.client(&endpoint.tls).await?;
        let mut request = client
            .request(if post { Method::POST } else { Method::GET }, url)
            .header(ACCEPT, "application/json")
            .timeout(endpoint.timeout);
        if post {
            request = request.form(&form);
        }
        let request = auth::apply(request, &endpoint.auth, self.secrets.as_ref()).await?;

        let response = request
            .send()
            .await
            .map_err(|e| errors::from_transport(e, endpoint.timeout))?;
        let status = response.status().as_u16();
        let body = read_body(response, endpoint.max_response_bytes, endpoint.timeout).await?;

        decode(status, &body)
    }
}

/// Reads at most `limit` bytes of the body; a longer one is a bad response, not a stall.
async fn read_body(
    mut response: Response,
    limit: usize,
    timeout: Duration,
) -> Result<Vec<u8>, PromqlError> {
    let too_large = || {
        PromqlError::BadResponse(format!(
            "the response is larger than the {limit} byte limit; narrow the query"
        ))
    };
    let declared = response
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<usize>().ok());
    if declared.is_some_and(|n| n > limit) {
        return Err(too_large());
    }
    let mut body = Vec::with_capacity(declared.unwrap_or(0).min(1 << 20));
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| errors::from_transport(e, timeout))?
    {
        if body.len() + chunk.len() > limit {
            return Err(too_large());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[async_trait]
impl PromqlEndpointPort for PromqlClient {
    async fn instant_query(
        &self,
        endpoint: &PromEndpoint,
        query: &str,
        at: Option<Timestamp>,
    ) -> Result<Vec<PromqlSeries>, PromqlError> {
        let params = at
            .map(|at| vec![("time", unix_seconds(at))])
            .unwrap_or_default();
        self.run(endpoint, Route::Instant, query, params).await
    }

    async fn range_query(
        &self,
        endpoint: &PromEndpoint,
        query: &str,
        range: TimeRange,
    ) -> Result<Vec<PromqlSeries>, PromqlError> {
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
        self.run(endpoint, Route::Range, query, params).await
    }
}

impl std::fmt::Debug for PromqlClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PromqlClient")
            .field("keychain", &self.secrets.is_some())
            .finish_non_exhaustive()
    }
}
