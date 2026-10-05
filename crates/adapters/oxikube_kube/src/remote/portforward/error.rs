//! Error mapping for port-forwarding.

use std::io;
use std::net::SocketAddr;

use oxikube_domain::OxiError;
use oxikube_domain::redact::redact;

use crate::auth::classify;

/// The longest server message kept for the UI, in characters.
const MAX_MESSAGE: usize = 300;

/// The failure to open the `portforward` websocket for `namespace/pod`.
///
/// kube reports any non-101 answer as a failed protocol switch carrying only the status
/// code, so the code is mapped here; everything else goes through [`classify`].
pub(super) fn open_error(err: &kube::Error, namespace: &str, pod: &str, port: u16) -> OxiError {
    use kube::client::UpgradeConnectionError::ProtocolSwitch;
    let kube::Error::UpgradeConnection(ProtocolSwitch(status)) = err else {
        return classify(err);
    };
    match status.as_u16() {
        401 => OxiError::auth(
            "the cluster rejected the credentials (they may have expired)",
            true,
        ),
        403 => OxiError::forbidden(format!(
            "not allowed to port-forward to pod {namespace}/{pod} (needs `create` on `pods/portforward`)"
        )),
        404 => OxiError::not_found(format!("pod {namespace}/{pod} not found")),
        400 | 422 => OxiError::validation(format!(
            "the cluster refused to forward port {port} of pod {namespace}/{pod} (is the pod running?)"
        )),
        408 | 504 => OxiError::timeout("the port-forward request to the cluster timed out"),
        code => OxiError::network(format!(
            "the cluster answered {code} instead of upgrading to a port-forward stream"
        )),
    }
}

/// An error message the pod's side sent on the port's error channel (the container refused
/// the connection, the port is not listening, ...).
pub(super) fn remote_failure(message: &str) -> OxiError {
    let redacted = redact(message);
    let line = redacted
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("");
    let line: String = line.trim().chars().take(MAX_MESSAGE).collect();
    OxiError::network(format!("the pod reported a port-forward error: {line}"))
}

/// Binding the local listener failed.
pub(super) fn bind_error(addr: SocketAddr, err: &io::Error) -> OxiError {
    match err.kind() {
        io::ErrorKind::AddrInUse => OxiError::conflict(format!(
            "local port {} on {} is already in use",
            addr.port(),
            addr.ip()
        )),
        io::ErrorKind::PermissionDenied => OxiError::validation(format!(
            "not allowed to listen on local port {} (ports below 1024 usually need privileges)",
            addr.port()
        )),
        io::ErrorKind::AddrNotAvailable => OxiError::validation(format!(
            "cannot listen on {}: the address does not belong to this machine",
            addr.ip()
        )),
        _ => OxiError::internal(format!("could not listen on {addr}: {err}")),
    }
}
