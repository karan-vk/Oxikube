//! What a failed `metrics.k8s.io` call means (ADR 0011).
//!
//! Absence is a result the UI renders ("metrics-server not installed", "no permission to read
//! metrics"), so the statuses that mean "this cluster cannot answer this user" become a
//! [`MissingReason`]: 404 `NotInstalled`, 503 `ApiUnavailable`, 403 `Forbidden`. Everything else
//! is a failure of the call itself and goes through [`classify`](crate::auth::classify)
//! unchanged, so a 401 stays `Auth` and timeouts stay retryable `Timeout`.

use oxikube_domain::OxiError;
use oxikube_domain::metrics::MissingReason;

use crate::auth::classify;

/// Why a list could not be answered: absence of the API, or a failed call.
#[derive(Debug)]
pub(super) enum Stop {
    /// The cluster cannot answer; shown to the user as a state.
    Absent(MissingReason),
    /// The call failed.
    Failed(OxiError),
}

impl From<kube::Error> for Stop {
    fn from(err: kube::Error) -> Self {
        match &err {
            // No APIService for `metrics.k8s.io`: metrics-server is not installed.
            kube::Error::Api(status) if status.code == 404 => {
                Stop::Absent(MissingReason::NotInstalled)
            }
            // The APIService exists but its backend is not serving (pod starting, crashed,
            // failing availability checks): installed, not answering.
            kube::Error::Api(status) if status.code == 503 => {
                Stop::Absent(MissingReason::ApiUnavailable)
            }
            // RBAC denies `metrics.k8s.io`: a visible state, not a hard error.
            kube::Error::Api(status) if status.code == 403 => {
                Stop::Absent(MissingReason::Forbidden)
            }
            _ => Stop::Failed(classify(&err)),
        }
    }
}
