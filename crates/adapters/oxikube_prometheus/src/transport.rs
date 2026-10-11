//! The pooled `reqwest` clients, one per TLS configuration.
//!
//! A `reqwest::Client` owns the connection pool and is expensive to build (it loads the platform
//! root store), so [`ClientPool`] builds one per distinct [`PromTls`] and reuses it. Building runs
//! on the blocking pool; nothing here blocks the caller's executor thread.

use std::collections::HashMap;
use std::time::Duration;

use oxikube_ports::{PromTls, PromqlError};
use parking_lot::Mutex;
use reqwest::{Certificate, Client, redirect};

/// How long an idle pooled connection is kept.
const POOL_IDLE_TIMEOUT: Duration = Duration::from_secs(60);
/// Redirects followed (an auth proxy or a trailing-slash fix may redirect once).
const MAX_REDIRECTS: usize = 5;

/// Clients keyed by the TLS settings that shape them.
#[derive(Default)]
pub(crate) struct ClientPool {
    clients: Mutex<HashMap<PromTls, Client>>,
}

impl ClientPool {
    /// The client for `tls`, built on first use.
    pub(crate) async fn client(&self, tls: &PromTls) -> Result<Client, PromqlError> {
        if let Some(client) = self.clients.lock().get(tls) {
            return Ok(client.clone());
        }
        let for_build = tls.clone();
        let client = tokio::task::spawn_blocking(move || build(&for_build))
            .await
            .map_err(|e| PromqlError::InvalidEndpoint(format!("client build failed: {e}")))??;
        self.clients.lock().insert(tls.clone(), client.clone());
        Ok(client)
    }
}

fn build(tls: &PromTls) -> Result<Client, PromqlError> {
    let mut builder = Client::builder()
        .use_rustls_tls()
        .redirect(redirect::Policy::limited(MAX_REDIRECTS))
        .pool_idle_timeout(POOL_IDLE_TIMEOUT)
        .user_agent(concat!("oxikube/", env!("CARGO_PKG_VERSION")));
    if let Some(pem) = &tls.ca_pem {
        let certificates = Certificate::from_pem_bundle(pem).map_err(|e| {
            PromqlError::InvalidEndpoint(format!("the CA certificate is not valid PEM: {e}"))
        })?;
        if certificates.is_empty() {
            return Err(PromqlError::InvalidEndpoint(
                "the CA bundle holds no certificate".into(),
            ));
        }
        for certificate in certificates {
            builder = builder.add_root_certificate(certificate);
        }
    }
    if tls.insecure_skip_verify {
        // Once per pooled client, not per request: a poller would repeat it forever.
        tracing::warn!(
            target: "oxikube_prometheus",
            "TLS verification is disabled for a Prometheus endpoint (insecure_skip_verify)"
        );
        builder = builder.danger_accept_invalid_certs(true);
    }
    builder
        .build()
        .map_err(|e| PromqlError::InvalidEndpoint(format!("cannot build the HTTP client: {e}")))
}
