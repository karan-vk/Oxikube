//! Error mapping for the resource data plane.
//!
//! Everything goes through [`classify`](crate::auth::classify); this module adds the one
//! distinction the port taxonomy cannot carry: a *stale list* (HTTP 410 Gone), which means
//! "start the list again", not "the object conflicts".

use std::fmt;

use oxikube_domain::{ErrorKind, OxiError};

use crate::auth::classify;

/// Marker attached (as the error source) to the `Conflict` error produced by HTTP 410 on a
/// list: an expired continue token, or an `Exact` resource version the server compacted.
///
/// Check with [`is_list_expired`]; the fix is to restart the list (what
/// [`KubeResources::list_all`](super::KubeResources::list_all) does) or relist for a watch.
#[derive(Debug)]
pub struct ListExpired;

impl fmt::Display for ListExpired {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the list's continue token or resource version expired (HTTP 410 Gone)")
    }
}

impl std::error::Error for ListExpired {}

/// Whether `err` came from a 410 Gone on a list call, so restarting the list can succeed.
pub fn is_list_expired(err: &OxiError) -> bool {
    use std::error::Error;
    err.kind() == ErrorKind::Conflict
        && err
            .source()
            .is_some_and(|source| source.downcast_ref::<ListExpired>().is_some())
}

/// Maps a failed list call: [`classify`], plus the [`ListExpired`] marker on 410.
pub(super) fn list_error(err: &kube::Error) -> OxiError {
    match err {
        kube::Error::Api(status) if status.code == 410 => {
            classify(err).with_source(ListExpired).with_retryable(true)
        }
        _ => classify(err),
    }
}

/// Maps a failed `get`.
pub(super) fn get_error(err: &kube::Error) -> OxiError {
    classify(err)
}

/// A server object that did not decode into a `Resource`.
pub(super) fn bad_object(what: &str, err: impl fmt::Display) -> OxiError {
    OxiError::internal(format!("the cluster sent an invalid {what}: {err}"))
}
