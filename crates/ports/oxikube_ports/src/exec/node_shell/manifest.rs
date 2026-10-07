//! The privileged pod behind a node shell, and the command exec'd in it.

use oxikube_domain::{OxiError, OxiResult};
use serde_json::{Map, Value, json};

use super::spec::{DEFAULT_NSENTER_ARGS, NodeShellSpec, NodeShellToleration};

/// Label on every node-shell pod: what the leftover sweep selects on.
pub const NODE_SHELL_LABEL: &str = "oxikube.dev/node-shell";

/// Annotation with the node the pod serves (a node name can be longer than a label value).
pub const NODE_ANNOTATION: &str = "oxikube.dev/node";

/// Annotation a live node shell keeps current with the time (Unix seconds) it was last seen
/// alive. A shell's owner refreshes it every minute or so; the leftover sweep spares any pod
/// whose newest stamp (this one, or its creation) is recent, so it only deletes pods whose owner
/// stopped (crashed, quit, lost its connection) and never another window's or user's live shell.
pub const HEARTBEAT_ANNOTATION: &str = "oxikube.dev/node-shell-heartbeat";

/// The shell container's name in the pod.
pub const CONTAINER_NAME: &str = "shell";

/// The image pull policies Kubernetes knows.
const PULL_POLICIES: [&str; 3] = ["Always", "IfNotPresent", "Never"];

/// A name that cannot change a request's route: non-empty, no `/`, `?`, `#`, `%` or whitespace.
fn plain_name(what: &str, value: &str) -> OxiResult<()> {
    let bad = |c: char| matches!(c, '/' | '?' | '#' | '%') || c.is_whitespace();
    if value.is_empty() || value.contains(bad) {
        return Err(OxiError::validation(format!(
            "{what} must be a plain name, not {value:?}"
        )));
    }
    Ok(())
}

fn toleration_json(toleration: &NodeShellToleration) -> Value {
    let mut out = Map::new();
    let text = [
        ("key", &toleration.key),
        ("operator", &toleration.operator),
        ("value", &toleration.value),
        ("effect", &toleration.effect),
    ];
    for (name, value) in text {
        if let Some(value) = value {
            out.insert(name.to_owned(), json!(value));
        }
    }
    if let Some(seconds) = toleration.toleration_seconds {
        out.insert("tolerationSeconds".to_owned(), json!(seconds));
    }
    Value::Object(out)
}

/// The pod that opens a shell on `spec.node`: privileged, in the host's PID, network and IPC
/// namespaces, pinned to the node by name (no scheduler, so it lands on a cordoned node too) and
/// tolerating what `spec.tolerations` lists (every taint by default). It only sleeps; the shell
/// is an exec of `nsenter` into the node's namespaces (see [`node_shell_command`]). Named by the
/// server (`generateName`).
///
/// This is the object the guard dry-runs and audits before the pod is created, and the one the
/// adapter then creates.
///
/// # Errors
///
/// `Validation` when the node or namespace is not a plain name, the image is blank, the pull
/// policy is not one Kubernetes knows, or an `nsenter` option is blank.
pub fn node_shell_manifest(spec: &NodeShellSpec) -> OxiResult<Value> {
    plain_name("a node name", &spec.node)?;
    plain_name("a namespace", &spec.namespace)?;
    if spec.image.trim().is_empty() {
        return Err(OxiError::validation("the node shell image is blank"));
    }
    if let Some(policy) = &spec.image_pull_policy
        && !PULL_POLICIES.contains(&policy.as_str())
    {
        return Err(OxiError::validation(format!(
            "the node shell image pull policy must be one of {}",
            PULL_POLICIES.join(", ")
        )));
    }
    if spec.nsenter_args.iter().any(|arg| arg.trim().is_empty()) {
        return Err(OxiError::validation("a node shell nsenter option is blank"));
    }
    let lifetime = spec.max_lifetime.as_secs().max(1);
    let mut container = json!({
        "name": CONTAINER_NAME,
        "image": spec.image,
        "command": ["sleep", lifetime.to_string()],
        "securityContext": {"privileged": true},
    });
    if let Some(policy) = &spec.image_pull_policy {
        container["imagePullPolicy"] = json!(policy);
    }
    let mut pod_spec = json!({
        "nodeName": spec.node,
        "hostPID": true,
        "hostNetwork": true,
        "hostIPC": true,
        "restartPolicy": "Never",
        "terminationGracePeriodSeconds": 0,
        "activeDeadlineSeconds": lifetime,
        "tolerations": spec.tolerations.iter().map(toleration_json).collect::<Vec<_>>(),
        "containers": [container],
    });
    if let Some(secret) = &spec.image_pull_secret {
        pod_spec["imagePullSecrets"] = json!([{"name": secret}]);
    }
    // The user's labels first: Oxikube's own win, so the sweep always finds the pod.
    let mut labels = Map::new();
    for (key, value) in &spec.labels {
        labels.insert(key.clone(), json!(value));
    }
    labels.insert("app.kubernetes.io/managed-by".to_owned(), json!("oxikube"));
    labels.insert(NODE_SHELL_LABEL.to_owned(), json!("true"));
    Ok(json!({
        "apiVersion": "v1",
        "kind": "Pod",
        "metadata": {
            "generateName": "oxikube-node-shell-",
            "namespace": spec.namespace,
            "labels": labels,
            "annotations": {NODE_ANNOTATION: spec.node},
        },
        "spec": pod_spec,
    }))
}

/// The command exec'd in the shell container: `nsenter` into the node's init process (the
/// options of `spec.nsenter_args`, by default every namespace), then the configured shell, by
/// default `bash -l` if the node has one, else `sh -l`.
pub fn node_shell_command(spec: &NodeShellSpec) -> Vec<String> {
    let mut command = vec!["nsenter".to_owned()];
    if spec.nsenter_args.is_empty() {
        command.extend(DEFAULT_NSENTER_ARGS.map(String::from));
    } else {
        command.extend(spec.nsenter_args.iter().cloned());
    }
    command.push("--".to_owned());
    if spec.shell.is_empty() {
        command.extend(
            [
                "sh",
                "-c",
                "if command -v bash >/dev/null 2>&1; then exec bash -l; else exec sh -l; fi",
            ]
            .map(String::from),
        );
    } else {
        command.extend(spec.shell.iter().cloned());
    }
    command
}

#[cfg(test)]
mod tests;
