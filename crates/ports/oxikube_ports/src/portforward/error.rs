//! [`PortForwardError`]: why a forward could not start or stopped working.

use std::net::SocketAddr;

use oxikube_domain::OxiError;

/// First port that needs no privileges on Linux and macOS.
pub const FIRST_UNPRIVILEGED_PORT: u16 = 1024;

/// The reasons a forward fails, in terms the UI can act on.
///
/// Adapters map their native errors into this taxonomy in one place; the manager and the
/// dialog show [`suggestion`](Self::suggestion) next to the message.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PortForwardError {
    /// The pod does not exist (any more) or is in a terminal phase.
    #[error("pod {0} is gone")]
    PodGone(String),
    /// The pod exists but cannot serve yet (pending, terminating, not running).
    #[error("pod {0} is not ready to forward")]
    PodNotReady(String),
    /// The container is not listening on the remote port (the pod reported a refusal).
    #[error("nothing is listening on remote port {port}: {detail}")]
    PortClosed {
        /// The remote port that refused.
        port: u16,
        /// The pod's one-line explanation (already redacted).
        detail: String,
    },
    /// The local listener could not be bound.
    #[error("cannot listen on {addr}: {}", bind_reason(*kind))]
    BindFailed {
        /// The address that was requested.
        addr: SocketAddr,
        /// What the OS answered: `AddrInUse`, `PermissionDenied`, `AddrNotAvailable`, ...
        kind: std::io::ErrorKind,
    },
    /// RBAC denies `create` on `pods/portforward`.
    #[error("not allowed to port-forward: {0}")]
    Forbidden(String),
    /// The connection to the cluster dropped or timed out; a retry may succeed.
    #[error("the connection to the cluster was lost: {0}")]
    Disconnected(String),
    /// Anything else.
    #[error("{0}")]
    Other(String),
}

fn bind_reason(kind: std::io::ErrorKind) -> &'static str {
    match kind {
        std::io::ErrorKind::AddrInUse => "the port is already in use",
        std::io::ErrorKind::PermissionDenied => "permission denied",
        std::io::ErrorKind::AddrNotAvailable => "the address does not belong to this machine",
        _ => "the listener could not be bound",
    }
}

impl PortForwardError {
    /// A one-line hint on what to do next, when there is one.
    ///
    /// * port in use: pick another port, or let Oxikube choose (local port `0`);
    /// * privileged port (below 1024): use a port from 1024 up;
    /// * forbidden: the role needs `create` on `pods/portforward`.
    pub fn suggestion(&self) -> Option<String> {
        use std::io::ErrorKind;
        match self {
            Self::BindFailed { addr, kind } => match kind {
                ErrorKind::AddrInUse => Some(format!(
                    "Port {} is in use: try another port, or choose automatically.",
                    addr.port()
                )),
                ErrorKind::PermissionDenied if addr.port() < FIRST_UNPRIVILEGED_PORT => {
                    Some(format!(
                        "Ports below {FIRST_UNPRIVILEGED_PORT} need elevated privileges: use {FIRST_UNPRIVILEGED_PORT} or higher."
                    ))
                }
                ErrorKind::PermissionDenied => Some(
                    "The OS refused the listener: try another port, or choose automatically."
                        .to_owned(),
                ),
                ErrorKind::AddrNotAvailable => {
                    Some("Bind to 127.0.0.1, or to an address of this machine.".to_owned())
                }
                _ => None,
            },
            Self::Forbidden(_) => Some(
                "Your role needs `create` on `pods/portforward` in this namespace (RBAC)."
                    .to_owned(),
            ),
            Self::PortClosed { port, .. } => {
                Some(format!("Check that the container listens on port {port}."))
            }
            Self::PodNotReady(_) => Some("Wait for the pod to be running, then retry.".to_owned()),
            Self::PodGone(_) => {
                Some("Pick the pod's replacement, or forward to its Service.".to_owned())
            }
            Self::Disconnected(_) | Self::Other(_) => None,
        }
    }

    /// Whether trying again unchanged can succeed.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::PodNotReady(_) | Self::Disconnected(_) | Self::PodGone(_)
        )
    }
}

impl From<PortForwardError> for OxiError {
    fn from(err: PortForwardError) -> Self {
        use std::io::ErrorKind;
        let message = err.to_string();
        match err {
            PortForwardError::PodGone(_) => OxiError::not_found(message),
            PortForwardError::PodNotReady(_) => OxiError::conflict(message),
            PortForwardError::PortClosed { .. } | PortForwardError::Disconnected(_) => {
                OxiError::network(message)
            }
            PortForwardError::BindFailed { kind, .. } => match kind {
                ErrorKind::AddrInUse => OxiError::conflict(message),
                ErrorKind::PermissionDenied | ErrorKind::AddrNotAvailable => {
                    OxiError::validation(message)
                }
                _ => OxiError::internal(message),
            },
            PortForwardError::Forbidden(_) => OxiError::forbidden(message),
            PortForwardError::Other(_) => OxiError::internal(message),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::ErrorKind;

    fn addr(port: u16) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], port))
    }

    #[test]
    fn bind_failures_suggest_the_right_fix() {
        let in_use = PortForwardError::BindFailed {
            addr: addr(8080),
            kind: ErrorKind::AddrInUse,
        };
        assert!(in_use.suggestion().unwrap().contains("another port"));
        let privileged = PortForwardError::BindFailed {
            addr: addr(80),
            kind: ErrorKind::PermissionDenied,
        };
        assert!(privileged.suggestion().unwrap().contains("1024"));
        assert!(privileged.to_string().contains("127.0.0.1:80"));
    }

    #[test]
    fn forbidden_names_the_rbac_resource() {
        let err = PortForwardError::Forbidden("pod ns/web".into());
        assert!(err.suggestion().unwrap().contains("pods/portforward"));
        assert_eq!(
            OxiError::from(err).kind(),
            oxikube_domain::ErrorKind::Forbidden
        );
    }

    #[test]
    fn conversion_to_oxi_error_keeps_the_kind() {
        use oxikube_domain::ErrorKind as K;
        let cases = [
            (PortForwardError::PodGone("p".into()), K::NotFound),
            (PortForwardError::PodNotReady("p".into()), K::Conflict),
            (
                PortForwardError::PortClosed {
                    port: 80,
                    detail: "refused".into(),
                },
                K::Network,
            ),
            (PortForwardError::Disconnected("x".into()), K::Network),
            (PortForwardError::Other("x".into()), K::Internal),
        ];
        for (err, kind) in cases {
            assert_eq!(OxiError::from(err).kind(), kind);
        }
    }

    #[test]
    fn retryable_kinds() {
        assert!(PortForwardError::Disconnected("x".into()).is_retryable());
        assert!(!PortForwardError::Forbidden("x".into()).is_retryable());
    }
}
