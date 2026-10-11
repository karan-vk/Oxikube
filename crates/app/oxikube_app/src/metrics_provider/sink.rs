//! The query side of a provider run: what was found and where it goes.

use oxikube_domain::ids::ClusterId;
use oxikube_ports::{DetectedProvider, PromEndpoint};

/// A provider that answered and the endpoint queries to it go to (for a configured endpoint, with
/// the credentials' keychain references; for a detected one, a Service reached through the API
/// server).
#[derive(Debug, Clone)]
pub struct Resolved {
    /// The provider, as the page shows it.
    pub provider: DetectedProvider,
    /// Where to send queries.
    pub endpoint: PromEndpoint,
}

/// Where the provider that answered goes: the query side. The binary implements it over
/// `oxikube_prometheus::PrometheusPort`, so the endpoint a query uses is set in the same step as
/// the state a view shows. `None` forgets the cluster's endpoint.
///
/// The service calls it while holding its state lock, so a newer decision is never overwritten by
/// an older one. An implementation is a quick in-memory set: it must not block and must not call
/// back into the [`MetricsProviderService`](super::MetricsProviderService).
pub trait EndpointSink: Send + Sync {
    /// `cluster`'s provider is now `resolved`, or none.
    fn publish(&self, cluster: &ClusterId, resolved: Option<&Resolved>);
}

/// An [`EndpointSink`] that drops everything (tests, and a build without a query side).
#[derive(Debug, Clone, Copy, Default)]
pub struct NoSink;

impl EndpointSink for NoSink {
    fn publish(&self, _: &ClusterId, _: Option<&Resolved>) {}
}
