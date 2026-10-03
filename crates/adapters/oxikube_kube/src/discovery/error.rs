//! Minimal `kube::Error` -> `OxiError` mapping for discovery.
//!
//! TODO(E03-S04): delete this file and call `auth::classify(&kube::Error)` instead once that
//! lands; this follows the table in `docs/ARCHITECTURE.md` and nothing more.

use std::error::Error as StdError;
use std::io;

use oxikube_domain::{ErrorKind, OxiError};

/// Maps a kube error from a discovery request. `what` names the request for the message.
///
/// Messages are fixed strings plus the API server's own status message; credential-bearing
/// errors (`kube::Error::Auth`) carry no source.
pub(super) fn from_kube(err: kube::Error, what: &str) -> OxiError {
    match err {
        kube::Error::Api(status) => {
            let message = format!("{what}: {}", status.message);
            let mapped = match status.code {
                401 => OxiError::auth(message, false),
                403 => OxiError::forbidden(message),
                404 => OxiError::not_found(message),
                400 | 422 => OxiError::validation(message),
                409 => OxiError::conflict(message),
                406 | 415 | 501 => OxiError::unsupported(message),
                429 | 502 | 503 | 504 => OxiError::network(message),
                _ => OxiError::internal(message),
            };
            mapped.with_source(kube::Error::Api(status))
        }
        kube::Error::Auth(_) => OxiError::auth(format!("{what}: credentials rejected"), false),
        kube::Error::HyperError(_) | kube::Error::Service(_) if is_timeout(&err) => {
            timeout(what, err)
        }
        kube::Error::HyperError(_)
        | kube::Error::Service(_)
        | kube::Error::RustlsTls(_)
        | kube::Error::TlsRequired => {
            OxiError::network(format!("{what}: connection failed")).with_source(err)
        }
        kube::Error::Discovery(_) => {
            OxiError::unsupported(format!("{what}: API discovery incomplete")).with_source(err)
        }
        err => OxiError::internal(format!("{what}: unexpected response")).with_source(err),
    }
}

fn timeout(what: &str, err: kube::Error) -> OxiError {
    OxiError::timeout(format!("{what}: request timed out")).with_source(err)
}

/// Whether anything in the error's source chain is a timeout.
fn is_timeout(err: &(dyn StdError + 'static)) -> bool {
    let mut current: Option<&(dyn StdError + 'static)> = Some(err);
    while let Some(e) = current {
        if e.downcast_ref::<io::Error>()
            .is_some_and(|io| io.kind() == io::ErrorKind::TimedOut)
            || e.to_string().contains("timed out")
        {
            return true;
        }
        current = e.source();
    }
    false
}

/// Whether the legacy per-group endpoints could succeed where aggregated discovery failed. False
/// for failures that would repeat identically (bad credentials, no permission, no connection).
pub(super) fn legacy_may_help(err: &OxiError) -> bool {
    !matches!(
        err.kind(),
        ErrorKind::Auth | ErrorKind::Forbidden | ErrorKind::Network | ErrorKind::Timeout
    )
}
