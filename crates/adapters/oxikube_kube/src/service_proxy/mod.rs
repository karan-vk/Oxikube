//! `ServiceProxyPort` over the API server's service proxy (E13-S05).
//!
//! `GET|POST /api/v1/namespaces/<ns>/services/[https:]<name>:<port>/proxy/<path>` asks the API
//! server to forward the request to a ready endpoint of the Service. It needs no local listener,
//! carries the connection's own credentials and works wherever the user may use `services/proxy`
//! (see the Kubernetes docs, "Manually constructing apiserver proxy URLs"). Metrics code uses it to
//! reach an in-cluster Prometheus; nothing here is Prometheus-specific.
//!
//! | Piece | Where |
//! |---|---|
//! | the request path and the validation of what goes into it | `request` |
//! | the size-capped body read and the API server's `Status` answers | `response` |
//!
//! # Answers
//!
//! A status from the Service or from the API server itself is an `Ok(ProxyResponse)`, so the
//! caller can tell "the API server refused `services/proxy`" (`403`, with a message), "no ready
//! endpoint" (`503`) and "no such Service" (`404`) from the Service's own answers. Only a failure
//! to get any answer is an `Err`: a connection problem, the deadline, or a body over the limit.
//! The response is never logged; the log line has the Service, the path and the status.
//!
//! # Retries
//!
//! The connection's client retries `429`, `503` and `504` with backoff (kube's server retry, the
//! pool's default). That applies here too, so the API server's `503` for a Service with no ready
//! endpoint is retried until the request's deadline and then reported as a timeout whose message
//! says what the likely cause is, rather than as the `503` itself.

mod request;
mod response;
#[cfg(test)]
mod tests;

use async_trait::async_trait;
use kube::Client;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{ProxyRequest, ProxyResponse, ServiceProxyPort, ServiceRef};
use tracing::debug;

use crate::auth::classify;

/// Requests to Services through one connection's API server. Cheap to clone.
#[derive(Clone)]
pub struct KubeServiceProxy {
    client: Client,
}

impl KubeServiceProxy {
    /// Proxies through `client`, the connection to one cluster.
    pub fn new(client: Client) -> Self {
        Self { client }
    }
}

impl std::fmt::Debug for KubeServiceProxy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KubeServiceProxy").finish_non_exhaustive()
    }
}

#[async_trait]
impl ServiceProxyPort for KubeServiceProxy {
    async fn send(
        &self,
        service: &ServiceRef,
        proxy_request: ProxyRequest,
    ) -> OxiResult<ProxyResponse> {
        let built = request::build(service, &proxy_request)?;
        let limit = proxy_request.max_response_bytes;
        let exchange = async {
            let response = self.client.send(built).await.map_err(|e| classify(&e))?;
            response::read(response, limit).await
        };
        let answer = tokio::time::timeout(proxy_request.timeout, exchange)
            .await
            .map_err(|_| {
                OxiError::timeout(format!(
                    "{service} did not answer through the API server within {:?} (a Service \
                     with no ready endpoint gets 503 from the API server, which the client keeps \
                     retrying)",
                    proxy_request.timeout
                ))
            })??;
        debug!(
            target: "oxikube_kube::service_proxy",
            service = %service,
            path = %proxy_request.path,
            status = answer.status,
            bytes = answer.body.len(),
            "service proxy request"
        );
        Ok(answer)
    }
}
