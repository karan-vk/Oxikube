//! The privileged pod behind a node shell.

use oxikube_domain::{OxiError, OxiResult};
use serde_json::{Value, json};

use super::config::NodeShellConfig;
use crate::subresource::segment;

/// Label on every node-shell pod: what the leftover sweep selects on.
pub const NODE_SHELL_LABEL: &str = "oxikube.dev/node-shell";

/// Annotation with the node the pod serves (a node name can be longer than a label value).
pub const NODE_ANNOTATION: &str = "oxikube.dev/node";

/// The shell container's name in the pod.
pub(super) const CONTAINER: &str = "shell";

/// The pod that opens a shell on `node`: privileged, in the host's PID, network and IPC
/// namespaces, pinned to the node by name (no scheduler, so it lands on a cordoned node too)
/// and tolerating every taint. It only sleeps; the shell is an exec of `nsenter` into the
/// node's namespaces (see `exec_command`). Named by the server (`generateName`).
///
/// This is the object the guard shows and audits before the pod is created.
///
/// # Errors
///
/// `Validation` when the node or namespace is not a plain name, or the image is blank.
pub fn node_shell_manifest(node: &str, config: &NodeShellConfig) -> OxiResult<Value> {
    segment("a node name", node)?;
    segment("a namespace", &config.namespace)?;
    if config.image.trim().is_empty() {
        return Err(OxiError::validation("the node shell image is blank"));
    }
    let lifetime = config.max_lifetime.as_secs().max(1);
    let mut spec = json!({
        "nodeName": node,
        "hostPID": true,
        "hostNetwork": true,
        "hostIPC": true,
        "restartPolicy": "Never",
        "terminationGracePeriodSeconds": 0,
        "activeDeadlineSeconds": lifetime,
        "tolerations": [{"operator": "Exists"}],
        "containers": [{
            "name": CONTAINER,
            "image": config.image,
            "command": ["sleep", lifetime.to_string()],
            "securityContext": {"privileged": true},
        }],
    });
    if let Some(secret) = &config.image_pull_secret {
        spec["imagePullSecrets"] = json!([{"name": secret}]);
    }
    Ok(json!({
        "apiVersion": "v1",
        "kind": "Pod",
        "metadata": {
            "generateName": "oxikube-node-shell-",
            "namespace": config.namespace,
            "labels": {
                "app.kubernetes.io/managed-by": "oxikube",
                NODE_SHELL_LABEL: "true",
            },
            "annotations": {NODE_ANNOTATION: node},
        },
        "spec": spec,
    }))
}

/// The command exec'd in the shell container: `nsenter` into the node's init process
/// (mount, UTS, IPC, network and PID namespaces), then the configured shell.
pub(super) fn exec_command(config: &NodeShellConfig) -> Vec<String> {
    let mut command: Vec<String> = ["nsenter", "-t", "1", "-m", "-u", "-i", "-n", "-p", "--"]
        .map(String::from)
        .into();
    if config.shell.is_empty() {
        command.extend(
            [
                "sh",
                "-c",
                "if command -v bash >/dev/null 2>&1; then exec bash -l; else exec sh -l; fi",
            ]
            .map(String::from),
        );
    } else {
        command.extend(config.shell.iter().cloned());
    }
    command
}
