//! Bodies for the pod-only subresources: ephemeral containers (debug) and in-place resize.
//!
//! Both are strategic merge patches sent with `patch_subresource` on the pod, the way
//! `kubectl debug` and `kubectl patch --subresource=resize` do it.

use oxikube_ports::Patch;
use serde_json::{Map, Value, json};

/// An ephemeral container to add to a running pod (`kubectl debug`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EphemeralContainerSpec {
    /// Container name; unique within the pod.
    pub name: String,
    /// Image to run.
    pub image: String,
    /// Entrypoint and arguments; empty keeps the image's.
    pub command: Vec<String>,
    /// Container whose process namespace to share (`targetContainerName`).
    pub target_container: Option<String>,
    /// Keep stdin open.
    pub stdin: bool,
    /// Allocate a TTY.
    pub tty: bool,
}

/// The patch that adds `container` to a pod's `ephemeralcontainers` subresource. Unset
/// options are left out of the body, not sent as defaults.
pub fn ephemeral_container_patch(container: &EphemeralContainerSpec) -> Patch {
    let mut body = Map::new();
    body.insert("name".into(), json!(container.name));
    body.insert("image".into(), json!(container.image));
    if !container.command.is_empty() {
        body.insert("command".into(), json!(container.command));
    }
    if let Some(target) = &container.target_container {
        body.insert("targetContainerName".into(), json!(target));
    }
    if container.stdin {
        body.insert("stdin".into(), json!(true));
    }
    if container.tty {
        body.insert("tty".into(), json!(true));
    }
    Patch::strategic(json!({"spec": {"ephemeralContainers": [Value::Object(body)]}}))
}

/// New resources for one container of a pod (in-place resize, Kubernetes 1.33+). Quantities are
/// Kubernetes strings (`500m`, `256Mi`); leave a list empty to leave it unchanged.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResizeSpec {
    /// Container name.
    pub container: String,
    /// `resources.requests` as (resource, quantity), for example `("cpu", "500m")`.
    pub requests: Vec<(String, String)>,
    /// `resources.limits` as (resource, quantity).
    pub limits: Vec<(String, String)>,
}

/// The patch for a pod's `resize` subresource that applies `resize`.
pub fn resize_patch(resize: &ResizeSpec) -> Patch {
    let pairs = |list: &[(String, String)]| -> Value {
        list.iter().map(|(k, v)| (k.clone(), json!(v))).collect()
    };
    let mut resources = Map::new();
    if !resize.requests.is_empty() {
        resources.insert("requests".into(), pairs(&resize.requests));
    }
    if !resize.limits.is_empty() {
        resources.insert("limits".into(), pairs(&resize.limits));
    }
    Patch::strategic(json!({"spec": {"containers": [
        {"name": resize.container, "resources": Value::Object(resources)}
    ]}}))
}
