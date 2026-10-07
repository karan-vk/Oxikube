//! Failures of a node shell, in words that say what to change.

use oxikube_domain::{ErrorKind, OxiError};
use oxikube_ports::NodeShellSpec;

/// `error` from creating or starting the shell pod of `spec`, with what to do about it. The
/// kind and whether it can be retried are kept; the message names the namespace and image the
/// settings chose, never anything the session carried.
pub(super) fn explain(error: OxiError, spec: &NodeShellSpec) -> OxiError {
    let (namespace, image) = (&spec.namespace, &spec.image);
    let original = error.message().trim_end_matches('.').to_owned();
    let lower = original.to_ascii_lowercase();
    let advice = match error.kind() {
        ErrorKind::Forbidden if lower.contains("podsecurity") || lower.contains("violates") => {
            format!(
                "The cluster's pod security admission refused the privileged shell pod in \
                 namespace {namespace}. Use a namespace that allows privileged pods (the \
                 `node_shell.namespace` setting; kube-system usually does), or ask a cluster \
                 admin to label {namespace} with pod-security.kubernetes.io/enforce=privileged."
            )
        }
        ErrorKind::Forbidden if lower.contains("quota") => format!(
            "A resource quota in namespace {namespace} refused the shell pod. Free some \
             quota there or use another namespace (the `node_shell.namespace` setting)."
        ),
        ErrorKind::Forbidden => format!(
            "You are not allowed to create pods in namespace {namespace}, and a node shell \
             needs `create` on pods and on pods/exec there. Pick a namespace you may use \
             (the `node_shell.namespace` setting) or ask for access."
        ),
        ErrorKind::Conflict => format!(
            "Check that the node can pull {image} (the `node_shell_image` setting, and \
             `node_shell_pull_secret` for a private registry). The shell pod was deleted."
        ),
        ErrorKind::Timeout => format!(
            "The node may still be pulling {image}. Try again, or pick a smaller image \
             (the `node_shell_image` setting). The shell pod was deleted."
        ),
        _ => return error,
    };
    OxiError::new(error.kind(), format!("{advice} ({original})"))
        .with_retryable(error.is_retryable())
}
