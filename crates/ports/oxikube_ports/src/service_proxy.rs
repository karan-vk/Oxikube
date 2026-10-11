//! Reaching a Service through the API server's service proxy: [`ServiceProxyPort`].
//!
//! `GET|POST /api/v1/namespaces/<ns>/services/[https:]<name>:<port>/proxy/<path>` makes the API
//! server forward the request to one of the Service's ready endpoints. It needs no local
//! listener and no port-forward, and it carries the connection's own credentials, so it is the
//! default way the metrics code reaches an in-cluster Prometheus (E13-S05, ADR 0011).
//!
//! Only the minimum of HTTP crosses the port: a method, a path, parameters and the status, body
//! and content type that came back. What the body means (a Prometheus envelope, say) is the
//! caller's business; `oxikube_prometheus` decodes it.
//!
//! # Adapter
//!
//! Implemented by `oxikube_kube` (`KubeServiceProxy`, over the connection's pooled client).

use std::fmt;
use std::time::Duration;

use async_trait::async_trait;
use oxikube_domain::OxiResult;

/// How the API server speaks to the Service's pods.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ProxyScheme {
    /// Plain HTTP (the proxy's default; no prefix in the path).
    #[default]
    Http,
    /// TLS (`https:` in the path). The API server does not verify the pod's certificate.
    Https,
}

/// A Service port, by number or by its name in `spec.ports`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ServicePort {
    /// `spec.ports[].port`.
    Number(u16),
    /// `spec.ports[].name`.
    Name(String),
}

impl fmt::Display for ServicePort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ServicePort::Number(number) => write!(f, "{number}"),
            ServicePort::Name(name) => f.write_str(name),
        }
    }
}

/// A Service and the port on it to talk to.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ServiceRef {
    /// The Service's namespace.
    pub namespace: String,
    /// The Service's name.
    pub name: String,
    /// The port.
    pub port: ServicePort,
    /// Plain or TLS towards the pod.
    pub scheme: ProxyScheme,
}

impl ServiceRef {
    /// A plain-HTTP reference to `namespace/name` on `port`.
    pub fn new(namespace: impl Into<String>, name: impl Into<String>, port: ServicePort) -> Self {
        Self {
            namespace: namespace.into(),
            name: name.into(),
            port,
            scheme: ProxyScheme::Http,
        }
    }

    /// The same reference with `scheme`.
    #[must_use]
    pub fn with_scheme(mut self, scheme: ProxyScheme) -> Self {
        self.scheme = scheme;
        self
    }

    /// `namespace/name:port`, the form the `prometheus.path` setting uses.
    pub fn short(&self) -> String {
        format!("{}/{}:{}", self.namespace, self.name, self.port)
    }

    /// Reads `namespace/name:port` (a number or a port name), as the `prometheus.path` setting
    /// writes it. `https:` in front of the name selects TLS. `None` for any other shape.
    pub fn parse(text: &str) -> Option<Self> {
        let (namespace, rest) = text.trim().split_once('/')?;
        let (rest, scheme) = match rest.strip_prefix("https:") {
            Some(rest) => (rest, ProxyScheme::Https),
            None => (
                rest.strip_prefix("http:").unwrap_or(rest),
                ProxyScheme::Http,
            ),
        };
        let (name, port) = rest.split_once(':')?;
        let valid = |part: &str| {
            !part.is_empty()
                && part
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
        };
        if !valid(namespace) || !valid(name) || !valid(port) {
            return None;
        }
        let port = port
            .parse::<u16>()
            .map_or_else(|_| ServicePort::Name(port.to_owned()), ServicePort::Number);
        Some(Self {
            namespace: namespace.to_owned(),
            name: name.to_owned(),
            port,
            scheme,
        })
    }
}

impl fmt::Display for ServiceRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.scheme {
            ProxyScheme::Http => f.write_str(&self.short()),
            ProxyScheme::Https => write!(f, "{}/https:{}:{}", self.namespace, self.name, self.port),
        }
    }
}

/// The HTTP method of a proxied request. Both are reads of a query API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProxyMethod {
    /// `GET` with the parameters in the query string.
    #[default]
    Get,
    /// `POST` with the parameters as an `application/x-www-form-urlencoded` body (for a query
    /// too long for a URL).
    PostForm,
}

/// One request through the service proxy.
#[derive(Debug, Clone)]
pub struct ProxyRequest {
    /// `GET` or form `POST`.
    pub method: ProxyMethod,
    /// The path on the Service, starting with `/` (`/api/v1/query`).
    pub path: String,
    /// Query-string parameters (`GET`) or form fields (`POST`), unencoded.
    pub params: Vec<(String, String)>,
    /// The whole request's deadline.
    pub timeout: Duration,
    /// The largest response body accepted.
    pub max_response_bytes: usize,
}

impl ProxyRequest {
    /// A `GET` of `path` with the default limits.
    pub fn get(path: impl Into<String>) -> Self {
        Self {
            method: ProxyMethod::Get,
            path: path.into(),
            params: Vec::new(),
            timeout: Duration::from_secs(10),
            max_response_bytes: 32 * 1024 * 1024,
        }
    }
}

/// What came back through the proxy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxyResponse {
    /// The HTTP status: the Service's, or the API server's own when it refused or could not
    /// forward (see [`api_server_message`](Self::api_server_message)).
    pub status: u16,
    /// The `Content-Type` header, if any.
    pub content_type: Option<String>,
    /// The body, at most the request's `max_response_bytes`; empty when [`truncated`](Self::truncated).
    pub body: Vec<u8>,
    /// The body was longer than the request's `max_response_bytes`, so it was dropped.
    pub truncated: bool,
    /// Set when the answer is the API server's own `Status` object (`services/proxy is
    /// forbidden`, `no endpoints available for service`, `services "x" not found`) and not
    /// something the Service said: its `message`.
    pub api_server_message: Option<String>,
}

/// A GET or form POST to a Service through the API server.
///
/// # Effects
///
/// Read-only by contract: callers send it to read APIs (PromQL queries). The API server decides
/// whether the user may use `services/proxy`; the port does not guard it.
///
/// # Errors
///
/// A status from the Service or the API server is an `Ok` [`ProxyResponse`], never an error, so
/// the caller can tell "forbidden" from "not a Prometheus"; so is a body over the size limit
/// ([`truncated`](ProxyResponse::truncated)). `Err` is for failures to get any answer, or to
/// form the request: [`Network`](oxikube_domain::ErrorKind::Network) /
/// [`Timeout`](oxikube_domain::ErrorKind::Timeout) (retryable),
/// [`Auth`](oxikube_domain::ErrorKind::Auth) when the cluster rejects the connection's
/// credentials, [`Validation`](oxikube_domain::ErrorKind::Validation) for a service or path that
/// cannot be put in the proxy URL.
#[async_trait]
pub trait ServiceProxyPort: Send + Sync {
    /// Sends `request` to `service`. Dropping the future cancels the request.
    async fn send(&self, service: &ServiceRef, request: ProxyRequest) -> OxiResult<ProxyResponse>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_setting_form() {
        let service = ServiceRef::parse("monitoring/prometheus-operated:9090").unwrap();
        assert_eq!(service.namespace, "monitoring");
        assert_eq!(service.name, "prometheus-operated");
        assert_eq!(service.port, ServicePort::Number(9090));
        assert_eq!(service.scheme, ProxyScheme::Http);
        assert_eq!(service.short(), "monitoring/prometheus-operated:9090");

        let named = ServiceRef::parse("observability/mimir-query-frontend:http-metrics").unwrap();
        assert_eq!(named.port, ServicePort::Name("http-metrics".into()));

        let tls = ServiceRef::parse("openshift-monitoring/https:thanos-querier:web").unwrap();
        assert_eq!(tls.scheme, ProxyScheme::Https);
        assert_eq!(
            tls.to_string(),
            "openshift-monitoring/https:thanos-querier:web"
        );
    }

    #[test]
    fn rejects_other_shapes() {
        for bad in [
            "",
            "prometheus",
            "monitoring/prometheus",
            "/prometheus:9090",
            "monitoring/:9090",
            "monitoring/prom:",
            "monitoring/prom:90/90",
            "a b/prom:9090",
            "monitoring/prom/x:9090",
        ] {
            assert_eq!(ServiceRef::parse(bad), None, "{bad:?}");
        }
    }
}
