//! Error mapping for subresource calls: [`write_error`] for the kind and the structured detail,
//! plus the two cases specific to subresources.
//!
//! A 404 has two meanings here: the object is missing, or the kind does not serve the
//! subresource. The server does not say which in a way that holds for every kind (a built-in
//! kind answers `the server could not find the requested resource`, a CRD answers
//! `widgets.x "w" not found` for an undeclared subresource), so `request` asks the server: it
//! reads the object itself, and only if that succeeds is the 404 `Unsupported`.
//!
//! A budget-blocked eviction is HTTP 429 with a cause the server tags `DisruptionBudget`:
//!
//! ```json
//! {"reason": "TooManyRequests", "code": 429,
//!  "message": "Cannot evict pod as it would violate the pod's disruption budget.",
//!  "details": {"causes": [{"reason": "DisruptionBudget",
//!                          "message": "The disruption budget web needs 2 healthy pods and has 2 currently"}]}}
//! ```

use kube::core::Status;
use oxikube_domain::OxiError;
use oxikube_ports::Subresource;

use crate::auth::redacted_line;
use crate::mutate::write_error;

pub use oxikube_domain::EvictionBlocked;

/// The [`EvictionBlocked`] marker of an eviction error, `None` for every other error. A caller
/// that drains waits and retries when this is `Some`. (The marker type lives in the domain, so
/// the app can say "blocked by a disruption budget" without naming the adapter.)
pub fn eviction_blocked(err: &OxiError) -> Option<&EvictionBlocked> {
    err.eviction_blocked()
}

/// The error for a subresource the kind does not serve (the object exists, the path 404s).
pub(super) fn unsupported(what: &str, subresource: &Subresource) -> OxiError {
    OxiError::unsupported(format!(
        "{what} does not serve the `{subresource}` subresource on this cluster"
    ))
}

/// Maps a failed eviction of `namespace/pod`.
pub(super) fn evict_error(err: &kube::Error, namespace: &str, pod: &str) -> OxiError {
    let base = write_error(err);
    let kube::Error::Api(status) = err else {
        return base;
    };
    let Some(reason) = budget_reason(status) else {
        return base;
    };
    OxiError::network(format!(
        "evicting pod {namespace}/{pod} is blocked by a PodDisruptionBudget: {reason}"
    ))
    .with_retryable(true)
    .with_source(EvictionBlocked { reason })
}

/// The budget's explanation when `status` is a 429 refusing an eviction for a
/// PodDisruptionBudget.
fn budget_reason(status: &Status) -> Option<String> {
    if status.code != 429 {
        return None;
    }
    let cause = status
        .details
        .iter()
        .flat_map(|details| details.causes.iter())
        .find(|cause| cause.reason == "DisruptionBudget");
    let text = match cause {
        Some(cause) => &cause.message,
        None if status.message.contains("disruption budget") => &status.message,
        None => return None,
    };
    Some(redacted_line(text))
}
