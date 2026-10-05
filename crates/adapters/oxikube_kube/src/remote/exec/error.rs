//! Error mapping for exec and attach.
//!
//! kube reports a refused upgrade as only the HTTP status, so the status picks the kind and
//! the target names the message. 400 and 404 are ambiguous on this route (an unknown container
//! is a 400, a pod that is not running is a 400, a missing pod is a 404), so
//! [`refine`] asks the cluster what is there before settling on `NotFound`, `Conflict` or
//! `Validation`; that read happens only on this failure path.

use kube::client::UpgradeConnectionError;
use oxikube_domain::OxiError;

use super::pods::PodShape;
use crate::auth::classify;

/// What the failed request was for, for messages.
pub(super) struct Target<'a> {
    pub(super) namespace: &'a str,
    pub(super) pod: &'a str,
    pub(super) container: Option<&'a str>,
    /// `exec` or `attach`.
    pub(super) verb: &'static str,
}

/// The first mapping of a failure to open the stream. A 400 or 404 comes back as the pod or
/// container `NotFound`/`Validation` guess and is sharpened by [`refine`].
pub(super) fn open_error(err: &kube::Error, target: &Target<'_>) -> OxiError {
    let Target {
        namespace,
        pod,
        verb,
        ..
    } = target;
    let kube::Error::UpgradeConnection(upgrade) = err else {
        return classify(err);
    };
    let UpgradeConnectionError::ProtocolSwitch(status) = upgrade else {
        // A proxy or an API server that answers the upgrade but not with the streaming
        // protocol the client speaks.
        return OxiError::unsupported(format!(
            "the cluster did not accept the websocket streaming protocol for {verb} ({upgrade})"
        ));
    };
    match status.as_u16() {
        401 => OxiError::auth(
            "the cluster rejected the credentials (they may have expired)",
            true,
        ),
        403 => OxiError::forbidden(format!(
            "not allowed to {verb} in pod {namespace}/{pod} (needs `create` on `pods/{verb}`)"
        )),
        404 => OxiError::not_found(format!("pod {namespace}/{pod} not found")),
        400 | 422 => OxiError::validation(format!(
            "the cluster refused to {verb} in pod {namespace}/{pod}"
        )),
        408 | 504 => OxiError::timeout(format!("the {verb} request to the cluster timed out")),
        code => OxiError::network(format!(
            "the cluster answered {code} instead of upgrading to a {verb} stream"
        )),
    }
}

/// Sharpens a 400 or 404 with what the cluster has for the pod: `shape` is `None` when the
/// pod does not exist.
pub(super) fn refine(target: &Target<'_>, shape: Option<&PodShape>) -> OxiError {
    let Target {
        namespace,
        pod,
        container,
        verb,
    } = target;
    let Some(shape) = shape else {
        return OxiError::not_found(format!("pod {namespace}/{pod} not found"));
    };
    if let Some(name) = container {
        if !shape.containers.iter().any(|c| c == name) {
            return OxiError::not_found(format!(
                "container {name} not found in pod {namespace}/{pod}"
            ));
        }
    }
    if shape.deleting || !matches!(shape.phase.as_str(), "Running" | "Pending") {
        return OxiError::conflict(format!(
            "pod {namespace}/{pod} is {} and cannot {verb}",
            if shape.deleting {
                "terminating"
            } else {
                &shape.phase
            }
        ));
    }
    OxiError::conflict(format!(
        "the container in pod {namespace}/{pod} is not running yet, or has stopped"
    ))
}
