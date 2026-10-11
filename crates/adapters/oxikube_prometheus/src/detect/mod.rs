//! Provider auto-detection (E13-S05): find a cluster's Prometheus-compatible service.
//!
//! [`ProviderDetector`] implements `ProviderDetectorPort`. It lists the Services of the
//! namespaces the [`Catalogue`] names (read-only, `list services`), matches them against the
//! catalogue's entries, and confirms a match with a cheap probe query (`vector(1)`) sent through
//! the API server's service proxy, which needs no local listener. The first entry that answers
//! wins and gives the stack's [`MetricsProviderKind`], which the query catalogue (E13-S06) uses
//! for label names.
//!
//! | Piece | Where |
//! |---|---|
//! | the data: which services, ports and prefixes to try | `catalogue`, `catalogue.json` |
//! | listing a namespace, reading a Service's ports | `services` |
//! | the run: list, match, probe, verdict | `run` |
//!
//! # What the verdict says
//!
//! | Outcome | [`DetectionState`] |
//! |---|---|
//! | a candidate answered the probe | `Found { kind, service, path prefix }` |
//! | every namespace was listed and nothing matched | `NotFound` |
//! | the user may not list Services in a namespace and nothing was found elsewhere | `Failed(Forbidden)` |
//! | a Service matched a known name but did not answer | `Failed(ProbeFailed)` |
//! | a list failed for another reason | `Failed(Other)` |
//!
//! Every Service looked for is in [`DetectionResult::probes`], found or not, so the settings page
//! can explain a missing stack. If the service proxy is blocked the probe fails and the failure
//! says so; a managed port-forward fallback waits for the port-forward manager (E15).
//!
//! # Cost
//!
//! One list per candidate namespace (at most four at once) and one probe per matching Service,
//! one after the other, each bounded by the probe timeout. It runs once per connect or on
//! demand, off the UI thread; nothing is cached here (the app layer keeps the result).

mod catalogue;
mod run;
mod services;
#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use oxikube_ports::{
    DetectionResult, PromEndpoint, PromqlEndpointPort, PromqlError, ProviderDetectorPort,
    ResourceReader, ServiceProxyPort,
};

pub use catalogue::{Catalogue, Entry, SchemeEntry};

use crate::client::PromqlClient;
use crate::proxy::ProxyPromqlClient;

/// How long one probe query may take. Detection should come back quickly even when a Service
/// matches but nothing answers behind it.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Finds Prometheus-compatible services in a cluster: it lists, matches, probes and classifies (see the crate docs).
#[derive(Clone)]
pub struct ProviderDetector {
    catalogue: Arc<Catalogue>,
    direct: Arc<dyn PromqlEndpointPort>,
    probe_timeout: Duration,
}

impl ProviderDetector {
    /// A detector over the built-in catalogue; URL endpoints it is asked to check go through a
    /// plain HTTP client.
    pub fn new() -> Self {
        Self::with_client(Arc::new(PromqlClient::new()))
    }

    /// A detector whose checks of URL endpoints go through `direct` (it needs the keychain for
    /// endpoints with stored credentials).
    pub fn with_client(direct: Arc<dyn PromqlEndpointPort>) -> Self {
        Self {
            catalogue: Arc::new(Catalogue::builtin().clone()),
            direct,
            probe_timeout: PROBE_TIMEOUT,
        }
    }

    /// The same detector looking for the services of `catalogue` instead.
    #[must_use]
    pub fn with_catalogue(mut self, catalogue: Catalogue) -> Self {
        self.catalogue = Arc::new(catalogue);
        self
    }

    /// The same detector with another probe deadline.
    #[must_use]
    pub fn with_probe_timeout(mut self, timeout: Duration) -> Self {
        self.probe_timeout = timeout;
        self
    }

    /// The catalogue in use.
    pub fn catalogue(&self) -> &Catalogue {
        &self.catalogue
    }
}

impl Default for ProviderDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for ProviderDetector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderDetector")
            .field("entries", &self.catalogue.entries.len())
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl ProviderDetectorPort for ProviderDetector {
    async fn detect(
        &self,
        resources: Arc<dyn ResourceReader>,
        proxy: Arc<dyn ServiceProxyPort>,
    ) -> DetectionResult {
        let client = ProxyPromqlClient::new(proxy, self.direct.clone());
        self.run(resources.as_ref(), &client).await
    }

    async fn check(
        &self,
        proxy: Arc<dyn ServiceProxyPort>,
        endpoint: &PromEndpoint,
    ) -> Result<(), PromqlError> {
        let client = ProxyPromqlClient::new(proxy, self.direct.clone());
        client.ping(endpoint).await
    }
}
