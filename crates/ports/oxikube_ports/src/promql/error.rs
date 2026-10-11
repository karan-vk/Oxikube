//! [`PromqlError`]: why a PromQL call failed, in terms the UI can act on.
//!
//! The taxonomy exists so "there is no Prometheus here" ([`Unreachable`](PromqlError::Unreachable),
//! [`Timeout`](PromqlError::Timeout)) is never confused with "the query is wrong"
//! ([`BadQuery`](PromqlError::BadQuery)) or "you may not read it"
//! ([`Unauthorized`](PromqlError::Unauthorized), [`Forbidden`](PromqlError::Forbidden)).
//! Messages are written to be shown after `metrics unavailable: ` and never contain a credential.
//!
//! [`PromqlEndpointPort`](crate::PromqlEndpointPort) returns this type directly. The cluster-level
//! [`PromqlPort`](crate::PromqlPort) returns [`OxiError`]s that carry it as their source;
//! [`PromqlError::of`] reads it back.

use std::error::Error as _;
use std::time::Duration;

use oxikube_domain::OxiError;

/// A failed PromQL call.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PromqlError {
    /// Nothing answered: connection refused, no route, DNS failure, connection reset.
    #[error("prometheus is unreachable: {0}")]
    Unreachable(String),
    /// The request exceeded the endpoint's deadline.
    #[error("prometheus did not answer within {0:?}")]
    Timeout(Duration),
    /// HTTP 401, or the configured keychain entry holds no credential.
    #[error("prometheus rejected the credentials: {0}")]
    Unauthorized(String),
    /// HTTP 403: authenticated but not allowed to query.
    #[error("not allowed to query prometheus: {0}")]
    Forbidden(String),
    /// The server rejected the query or its parameters (HTTP 400 / 422, `errorType`
    /// `bad_data` / `execution`), or the request failed client-side validation.
    #[error("invalid query: {message}")]
    BadQuery {
        /// The server's explanation, e.g. the PromQL parse error.
        message: String,
    },
    /// The server failed (HTTP 5xx other than a query timeout) or throttled the request.
    #[error("prometheus server error ({status}): {message}")]
    ServerError {
        /// The HTTP status.
        status: u16,
        /// The server's `error` text, or the status reason.
        message: String,
    },
    /// The TLS handshake failed: untrusted or expired certificate, host name mismatch, protocol
    /// error.
    #[error("tls handshake failed: {0}")]
    TlsError(String),
    /// The endpoint answered but not with a Prometheus API response: not JSON, an unknown result
    /// type, a body over the size limit.
    #[error("unexpected response from prometheus: {0}")]
    BadResponse(String),
    /// The endpoint configuration cannot form a request (bad URL, unreadable CA certificate).
    #[error("invalid prometheus endpoint: {0}")]
    InvalidEndpoint(String),
}

impl PromqlError {
    /// `true` when the endpoint is simply not there (nothing answered in time): the normal "no
    /// Prometheus" state the UI shows as `metrics unavailable`.
    pub fn is_absent(&self) -> bool {
        matches!(self, Self::Unreachable(_) | Self::Timeout(_))
    }

    /// `true` when the endpoint is fine and the query is at fault.
    pub fn is_query_error(&self) -> bool {
        matches!(self, Self::BadQuery { .. })
    }

    /// `true` when retrying the same call later may succeed.
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Unreachable(_) | Self::Timeout(_) => true,
            Self::ServerError { status, .. } => matches!(status, 429 | 500 | 502 | 503 | 504),
            _ => false,
        }
    }

    /// The error carried by an [`OxiError`] returned from the cluster-level port, if it came from
    /// a PromQL call.
    pub fn of(error: &OxiError) -> Option<&PromqlError> {
        error.source()?.downcast_ref::<PromqlError>()
    }
}

impl From<PromqlError> for OxiError {
    /// Maps to the nearest [`ErrorKind`](oxikube_domain::ErrorKind) and keeps the original as the
    /// source, so [`PromqlError::of`] recovers it.
    fn from(error: PromqlError) -> Self {
        let message = error.to_string();
        let retryable = error.is_retryable();
        let base = match &error {
            PromqlError::Unreachable(_) | PromqlError::ServerError { .. } => {
                OxiError::network(message)
            }
            PromqlError::Timeout(_) => OxiError::timeout(message),
            PromqlError::Unauthorized(_) => OxiError::auth(message, false),
            PromqlError::Forbidden(_) => OxiError::forbidden(message),
            PromqlError::BadQuery { .. } | PromqlError::InvalidEndpoint(_) => {
                OxiError::validation(message)
            }
            PromqlError::TlsError(_) => OxiError::network(message),
            PromqlError::BadResponse(_) => OxiError::unsupported(message),
        };
        base.with_retryable(retryable).with_source(error)
    }
}

#[cfg(test)]
mod tests {
    use oxikube_domain::ErrorKind;

    use super::*;

    #[test]
    fn maps_to_oxi_kinds_and_round_trips() {
        let cases = [
            (
                PromqlError::Unreachable("refused".into()),
                ErrorKind::Network,
                true,
            ),
            (
                PromqlError::Timeout(Duration::from_secs(3)),
                ErrorKind::Timeout,
                true,
            ),
            (
                PromqlError::Unauthorized("401".into()),
                ErrorKind::Auth,
                false,
            ),
            (
                PromqlError::Forbidden("403".into()),
                ErrorKind::Forbidden,
                false,
            ),
            (
                PromqlError::BadQuery {
                    message: "x".into(),
                },
                ErrorKind::Validation,
                false,
            ),
            (
                PromqlError::ServerError {
                    status: 503,
                    message: "x".into(),
                },
                ErrorKind::Network,
                true,
            ),
            (
                PromqlError::ServerError {
                    status: 501,
                    message: "x".into(),
                },
                ErrorKind::Network,
                false,
            ),
            (
                PromqlError::TlsError("bad cert".into()),
                ErrorKind::Network,
                false,
            ),
            (
                PromqlError::BadResponse("html".into()),
                ErrorKind::Unsupported,
                false,
            ),
            (
                PromqlError::InvalidEndpoint("url".into()),
                ErrorKind::Validation,
                false,
            ),
        ];
        for (error, kind, retryable) in cases {
            let oxi = OxiError::from(error.clone());
            assert_eq!(oxi.kind(), kind, "{error:?}");
            assert_eq!(oxi.is_retryable(), retryable, "{error:?}");
            assert_eq!(PromqlError::of(&oxi), Some(&error));
        }
        assert_eq!(PromqlError::of(&OxiError::internal("x")), None);
    }

    #[test]
    fn absent_and_query_error_are_distinct() {
        assert!(PromqlError::Unreachable(String::new()).is_absent());
        assert!(
            !PromqlError::BadQuery {
                message: String::new()
            }
            .is_absent()
        );
        assert!(
            PromqlError::BadQuery {
                message: String::new()
            }
            .is_query_error()
        );
        assert!(!PromqlError::Unauthorized(String::new()).is_absent());
    }
}
