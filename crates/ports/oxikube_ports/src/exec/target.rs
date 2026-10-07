//! Descriptors the [`ExecPort`](super::ExecPort) methods take: plain data, no kube types.

use std::time::Duration;

use oxikube_domain::ids::ResourceRef;
use oxikube_domain::{OxiError, OxiResult};

/// Namespace and name of the pod a target names.
fn namespaced(pod: &ResourceRef) -> OxiResult<(&str, &str)> {
    match pod.namespace.as_deref() {
        Some(ns) if !ns.is_empty() => Ok((ns, &pod.name)),
        _ => Err(OxiError::validation(format!(
            "pod {} has no namespace",
            pod.name
        ))),
    }
}

/// A command to run in a container of a pod ([`ExecPort::exec`](super::ExecPort::exec)).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecTarget {
    /// The pod. Its cluster is the port's own; its namespace is required.
    pub pod: ResourceRef,
    /// Container; `None` means the pod's only (or default) container.
    pub container: Option<String>,
    /// argv, no shell. Must not be empty.
    pub command: Vec<String>,
    /// Allocate a TTY (an interactive terminal; resize works; stdout and stderr merge).
    pub tty: bool,
    /// Attach stdin.
    pub stdin: bool,
}

impl ExecTarget {
    /// An interactive session (TTY, stdin) running `command` in the default container.
    pub fn interactive(pod: ResourceRef, command: Vec<String>) -> Self {
        Self {
            pod,
            container: None,
            command,
            tty: true,
            stdin: true,
        }
    }

    /// Sets the container.
    #[must_use]
    pub fn container(mut self, container: impl Into<String>) -> Self {
        self.container = Some(container.into());
        self
    }

    /// `(namespace, pod name)`, or a `Validation` error when the pod has no namespace.
    ///
    /// # Errors
    ///
    /// `Validation` for a pod reference without a namespace.
    pub fn namespaced_pod(&self) -> OxiResult<(&str, &str)> {
        namespaced(&self.pod)
    }
}

/// A running container to attach to ([`ExecPort::attach`](super::ExecPort::attach)).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachTarget {
    /// The pod. Its cluster is the port's own; its namespace is required.
    pub pod: ResourceRef,
    /// Container; `None` means the pod's only (or default) container.
    pub container: Option<String>,
    /// Allocate a TTY.
    pub tty: bool,
    /// Attach stdin.
    pub stdin: bool,
}

impl AttachTarget {
    /// An interactive attach (TTY, stdin) to the default container.
    pub fn interactive(pod: ResourceRef) -> Self {
        Self {
            pod,
            container: None,
            tty: true,
            stdin: true,
        }
    }

    /// Sets the container.
    #[must_use]
    pub fn container(mut self, container: impl Into<String>) -> Self {
        self.container = Some(container.into());
        self
    }

    /// `(namespace, pod name)`, or a `Validation` error when the pod has no namespace.
    ///
    /// # Errors
    ///
    /// `Validation` for a pod reference without a namespace.
    pub fn namespaced_pod(&self) -> OxiResult<(&str, &str)> {
        namespaced(&self.pod)
    }
}

/// An ephemeral debug container to add to a pod
/// ([`ExecPort::create_debug_container`](super::ExecPort::create_debug_container)).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebugContainerSpec {
    /// The pod to debug. Its namespace is required.
    pub pod: ResourceRef,
    /// Name of the new container; `None` lets the adapter pick a unique one (`debugger-xxxxx`).
    pub name: Option<String>,
    /// Image to run.
    pub image: String,
    /// Container whose process namespace to share (`kubectl debug --target`).
    pub target_container: Option<String>,
    /// Entrypoint and arguments; empty keeps the image's.
    pub command: Vec<String>,
    /// How long to wait for the container to start (image pulls included).
    pub start_timeout: Duration,
}

impl DebugContainerSpec {
    /// A debug container running `image` in `pod`, with the defaults of `kubectl debug`.
    pub fn new(pod: ResourceRef, image: impl Into<String>) -> Self {
        Self {
            pod,
            name: None,
            image: image.into(),
            target_container: None,
            command: Vec::new(),
            start_timeout: Duration::from_secs(60),
        }
    }

    /// `(namespace, pod name)`, or a `Validation` error when the pod has no namespace.
    ///
    /// # Errors
    ///
    /// `Validation` for a pod reference without a namespace.
    pub fn namespaced_pod(&self) -> OxiResult<(&str, &str)> {
        namespaced(&self.pod)
    }
}
