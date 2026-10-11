//! `oxikube_prometheus` — layer: `adapters`.
//!
//! PromqlPort adapter: HTTP client, provider auto-detection (kube-prometheus, Lens stack,
//! VictoriaMetrics, Mimir, OpenShift) and per-provider query catalogues.
//!
//! This crate currently holds the HTTP client (E13-S04); detection and the query catalogues
//! arrive with E13-S05 and E13-S06.
//!
//! | Item | Role |
//! |---|---|
//! | [`PromqlClient`] | `PromqlEndpointPort` over HTTP: instant and range queries against any Prometheus-compatible API (Prometheus, VictoriaMetrics, Mimir, Thanos) |
//! | [`PrometheusPort`] | `PromqlPort` per cluster: the endpoint each cluster's metrics come from, plus the client |
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
mod errors;
mod transport;
mod url;
mod wire;

pub use client::PromqlClient;
pub use cluster::PrometheusPort;
