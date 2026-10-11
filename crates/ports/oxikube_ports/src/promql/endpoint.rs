//! [`PromEndpoint`]: where and how to reach one Prometheus-compatible HTTP API.
//!
//! The type is plain data, so settings (E13-S05), detection and the tests build it the same way.
//! Credentials are never stored as plain strings: a [`PromSecret`] is either an in-memory
//! [`SecretString`] (whose `Debug` prints a marker) or a [`SecretKey`] naming a keychain entry
//! that the adapter resolves through [`SecretStorePort`](crate::SecretStorePort) at request time.
//! Nothing here is ever written to disk or to a log.

use std::time::Duration;

use crate::secrets::{SecretKey, SecretString};
use crate::service_proxy::ServiceRef;

/// Default whole-request deadline (connect, send, wait and read the body).
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// Default cap on a response body. A range query over many series can be huge; past this the
/// request fails with [`PromqlError::BadResponse`](crate::PromqlError::BadResponse) instead of
/// stalling the poller or filling memory.
pub const DEFAULT_MAX_RESPONSE_BYTES: usize = 32 * 1024 * 1024;

/// A secret an endpoint authenticates with.
#[derive(Debug, Clone)]
pub enum PromSecret {
    /// Held in memory only (typed in this session, or read from the keychain by the caller).
    Memory(SecretString),
    /// A keychain entry, read through `SecretStorePort` when a request is made. Only the name is
    /// configured; a missing entry is [`PromqlError::Unauthorized`](crate::PromqlError::Unauthorized).
    Keychain(SecretKey),
}

/// How requests to the endpoint authenticate.
#[derive(Debug, Clone, Default)]
pub enum PromAuth {
    /// No `Authorization` header is sent.
    #[default]
    None,
    /// `Authorization: Bearer <token>`.
    Bearer(PromSecret),
    /// `Authorization: Basic base64(username:password)`.
    Basic {
        /// The user name (not secret).
        username: String,
        /// The password.
        password: PromSecret,
    },
}

/// TLS settings for an `https` endpoint. Ignored for plain `http`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct PromTls {
    /// Accept any server certificate and host name. Off by default; turning it on is an explicit
    /// per-endpoint choice and the adapter logs that it is on.
    pub insecure_skip_verify: bool,
    /// An extra root certificate (PEM, one or more certificates) trusted besides the platform's.
    pub ca_pem: Option<Vec<u8>>,
}

/// A Prometheus-compatible HTTP API: Prometheus, VictoriaMetrics, Mimir, Thanos.
///
/// The query URL is `base_url` + `path_prefix` + `/api/v1/query[_range]`, so
/// `https://mimir.example/` with prefix `/prometheus` queries
/// `https://mimir.example/prometheus/api/v1/query`. Slashes at the joins are normalised.
#[derive(Debug, Clone)]
pub struct PromEndpoint {
    /// Scheme, host, optional port and path, e.g. `http://prometheus.monitoring:9090`. No
    /// credentials in the URL; use [`auth`](Self::auth). Empty, and ignored, when
    /// [`service`](Self::service) is set.
    pub base_url: String,
    /// An in-cluster Service reached through the API server's service proxy instead of a URL
    /// (E13-S05). The request goes out with the cluster connection's own credentials, so
    /// [`auth`](Self::auth) and [`tls`](Self::tls) do not apply. Only a client that has the
    /// cluster's `ServiceProxyPort` can send to such an endpoint.
    pub service: Option<ServiceRef>,
    /// A path inserted before `/api/v1/...` (Mimir's `/prometheus`, OpenShift's thanos-querier,
    /// Grafana-fronted setups). `None` or empty adds nothing.
    pub path_prefix: Option<String>,
    /// Authentication.
    pub auth: PromAuth,
    /// TLS settings.
    pub tls: PromTls,
    /// Deadline for one request, body included.
    pub timeout: Duration,
    /// Largest response body accepted, in bytes.
    pub max_response_bytes: usize,
}

impl PromEndpoint {
    /// An endpoint at `base_url` with no auth, default TLS and the default limits.
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            service: None,
            path_prefix: None,
            auth: PromAuth::None,
            tls: PromTls::default(),
            timeout: DEFAULT_TIMEOUT,
            max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
        }
    }

    /// An endpoint on `service`, reached through the API server's service proxy, with the default
    /// limits.
    pub fn service(service: ServiceRef) -> Self {
        Self {
            service: Some(service),
            ..Self::new(String::new())
        }
    }

    /// Where the endpoint is, for a status line: the service (`ns/name:port`) or the URL.
    pub fn location(&self) -> String {
        match &self.service {
            Some(service) => service.to_string(),
            None => self.base_url.clone(),
        }
    }

    /// Sets the path prefix.
    pub fn with_path_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.path_prefix = Some(prefix.into());
        self
    }

    /// Sets the authentication.
    pub fn with_auth(mut self, auth: PromAuth) -> Self {
        self.auth = auth;
        self
    }

    /// Sets the TLS settings.
    pub fn with_tls(mut self, tls: PromTls) -> Self {
        self.tls = tls;
        self
    }

    /// Sets the request deadline.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Sets the response size cap.
    pub fn with_max_response_bytes(mut self, bytes: usize) -> Self {
        self.max_response_bytes = bytes;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_never_prints_a_secret() {
        let endpoint = PromEndpoint::new("https://prom.example").with_auth(PromAuth::Bearer(
            PromSecret::Memory(SecretString::from("s3cr3t-token".to_owned())),
        ));
        let text = format!("{endpoint:?}");
        assert!(!text.contains("s3cr3t-token"), "{text}");
        let basic = PromAuth::Basic {
            username: "bob".into(),
            password: PromSecret::Memory(SecretString::from("hunter2".to_owned())),
        };
        assert!(!format!("{basic:?}").contains("hunter2"));
    }

    #[test]
    fn defaults_are_bounded() {
        let endpoint = PromEndpoint::new("http://x");
        assert_eq!(endpoint.timeout, DEFAULT_TIMEOUT);
        assert!(endpoint.max_response_bytes > 0);
        assert!(!endpoint.tls.insecure_skip_verify);
    }
}
