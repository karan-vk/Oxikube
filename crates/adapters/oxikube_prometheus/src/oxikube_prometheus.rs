//! `oxikube_prometheus` — layer: `adapters`.
//!
//! PromqlPort adapter: HTTP client, provider auto-detection (kube-prometheus, Lens stack,
//! VictoriaMetrics, Mimir, OpenShift) and per-provider query catalogues.
//!
//! This crate holds the HTTP client (E13-S04), the provider detection (E13-S05) and the query
//! catalogue (E13-S06).
//!
//! | Item | Role |
//! |---|---|
//! | [`PromqlClient`] | `PromqlEndpointPort` over HTTP: instant and range queries against any Prometheus-compatible API (Prometheus, VictoriaMetrics, Mimir, Thanos) |
//! | [`ProxyPromqlClient`] | `PromqlEndpointPort` for an endpoint on a Service: the query goes through the cluster's API server service proxy (`ServiceProxyPort`), a URL goes to the direct client |
//! | [`PrometheusPort`] | `PromqlPort` per cluster: the endpoint each cluster's metrics come from, plus the clients ([`ServiceProxies`] finds a cluster's proxy per query) |
//! | [`PromqlCatalogue`] | `QueryCataloguePort`: the PromQL of a `MetricRequest` (cpu, memory, network, disk, pod count at cluster, node, namespace, pod, container, PVC and ingress scope) for each detected stack; pure string building, no I/O |
//! | [`ProviderDetector`] | `ProviderDetectorPort`: lists the Services of the namespaces a [`Catalogue`] names, probes the matches through the service proxy and classifies the stack |
//!
//! # Detection
//!
//! `detect/catalogue.json` is the data: per entry a stack, a Service name (or name prefix), the
//! namespaces to look in, ports by preference, a scheme and path prefixes to try. Adding a stack
//! is one entry. A run lists each namespace once, tries the entries in order and stops at the
//! first that answers a `vector(1)` query; every Service looked for is recorded with its
//! outcome, and a namespace that could not be listed is never read as "nothing there". Read-only.
//!
//! # Query catalogue
//!
//! `queries/` has one module per stack ([`MetricsProviderKind`](oxikube_domain::MetricsProviderKind)),
//! each documenting the series conventions it assumes, and one module per metric family that
//! builds the query. Names are escaped on their way into a selector; queries name their
//! namespace, pod, node or claim, never a regex over everything; the step and `rate()` window of
//! a range come from `oxikube_domain::metric_query::steps_for`. Golden files per provider are in
//! `tests/golden/queries/`.
//!
//! # Behaviour
//!
//! * Auth is none, bearer or basic; the secret is in memory or a keychain reference, resolved per
//!   request and never logged ([`PromAuth`](oxikube_ports::PromAuth)).
//! * The URL is `base_url` + `path_prefix` + `/api/v1/query[_range]`, slashes normalised.
//! * TLS is rustls with ring and the platform roots; a custom CA and an insecure toggle are
//!   per-endpoint. One pooled HTTP client per TLS configuration.
//! * Every request has a deadline and a response size cap, so a huge range query cannot stall a
//!   poller. Dropping the call's future closes the connection (cancel on drop).
//! * Failures are a [`PromqlError`](oxikube_ports::PromqlError): "no Prometheus", "not allowed",
//!   "bad query" and "server error" are different variants.
//!
//! See `README.md` in this crate and `docs/ARCHITECTURE.md` for the allowed
//! dependency direction. `cargo xtask lint-deps` enforces it.

mod auth;
mod client;
mod cluster;
mod decode;
mod detect;
mod errors;
mod proxy;
mod queries;
mod transport;
mod url;
mod wire;

pub use client::PromqlClient;
pub use cluster::{PrometheusPort, ServiceProxies};
pub use detect::{Catalogue, Entry, PROBE_TIMEOUT, ProviderDetector, SchemeEntry};
pub use proxy::ProxyPromqlClient;
pub use queries::PromqlCatalogue;
