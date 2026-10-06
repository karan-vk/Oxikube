//! [`WarningPort`]: the `Warning:` headers the API server attaches to its responses.
//!
//! # Adapter
//!
//! Implemented by `oxikube_kube`: a layer on the connection's HTTP client reads the `Warning`
//! response headers (RFC 7234, as the Kubernetes API server writes them: deprecated APIs,
//! unknown fields, admission notes) of every request the connection makes, redacts the text and
//! publishes it. The testkit's `FakeWarningPort` lets a test push one.
//!
//! # Contract
//!
//! [`subscribe`](WarningPort::subscribe) returns a live stream of the warnings that arrive after
//! the call. It is a broadcast: every subscriber sees every warning, nothing is replayed, and a
//! slow subscriber misses the oldest ones rather than slowing the connection down. The adapter
//! sends a warning per header value, in the order the server wrote them, and does not
//! de-duplicate: the app decides what to show once (`oxikube_app::store`, E07-S10).
//!
//! The text never carries credentials: the adapter passes it through the redaction module
//! before it crosses this port (non-negotiable 5).

use futures::stream::BoxStream;

/// One warning from the API server.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ApiWarning {
    /// The warning code of the header (`299`, "miscellaneous persistent warning", for every
    /// Kubernetes warning today).
    pub code: u16,
    /// The warning text, redacted ("v1 Endpoints is deprecated in v1.33+; use discovery.k8s.io/v1
    /// EndpointSlice").
    pub text: String,
}

impl ApiWarning {
    /// A warning with the usual code, `299`.
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            code: 299,
            text: text.into(),
        }
    }
}

/// A source of [`ApiWarning`]s for one cluster connection. See the [module docs](self).
pub trait WarningPort: Send + Sync {
    /// The warnings that arrive from now on, as a stream that ends when the connection does.
    /// Dropping the stream unsubscribes.
    fn subscribe(&self) -> BoxStream<'static, ApiWarning>;
}
