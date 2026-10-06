//! The API server's `Warning:` response headers, surfaced as [`WarningPort`] (E07-S10).
//!
//! The Kubernetes API server answers a request that touched a deprecated API, sent an unknown
//! field, or tripped an admission note with a `Warning: 299 - "text"` header (RFC 7234). `kubectl`
//! prints them on stderr; kube-rs drops them, and k9s hides them (k9s #4106). A [`WarningLayer`]
//! on the connection's HTTP client reads them off every response, redacts the text and publishes
//! it on a [`WarningHub`], one channel per kubeconfig context; the connector hands the channel
//! out as the connection's [`WarningPort`].
//!
//! | File | Holds |
//! |---|---|
//! | `header` | [`parse`]: one header value into [`ApiWarning`]s |
//! | `hub` | [`WarningHub`] (a broadcast per context), [`WarningSink`], [`HubPort`] |
//! | `layer` | [`WarningLayer`], the tower layer that reads the headers |
//!
//! The hub is process-wide ([`WarningHub::global`]) because clients are built by the pool, deep
//! below the connector that needs the port; the context name is the key between them.
//!
//! [`WarningPort`]: oxikube_ports::WarningPort
//! [`ApiWarning`]: oxikube_ports::ApiWarning

mod header;
mod hub;
mod layer;

pub use header::parse;
pub use hub::{HubPort, WarningHub, WarningSink};
pub use layer::{WarningLayer, WarningService};
