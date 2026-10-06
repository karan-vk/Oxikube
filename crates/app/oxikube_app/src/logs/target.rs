//! [`LogTarget`]: what a log session reads.

use std::fmt;

use oxikube_domain::ids::ResourceRef;

/// One container of one pod: the key a [`LogSession`](super::LogSession) is opened and listed by.
///
/// A target with no container reads the pod's only (or default) container. A label selector
/// (Deployment, StatefulSet, ...) is not a target of its own: the aggregation (E08-S04) resolves
/// it to pods and opens one session per pod, merging them.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LogTarget {
    /// Namespace of the pod.
    pub namespace: String,
    /// Pod name.
    pub pod: String,
    /// Container name; `None` for the pod's only (or default) container.
    pub container: Option<String>,
}

impl LogTarget {
    /// A pod's default container.
    pub fn pod(namespace: impl Into<String>, pod: impl Into<String>) -> Self {
        Self {
            namespace: namespace.into(),
            pod: pod.into(),
            container: None,
        }
    }

    /// The default container of the namespaced object `pod` refers to. `None` for a
    /// cluster-scoped reference, which names no pod.
    pub fn of(pod: &ResourceRef) -> Option<Self> {
        let namespace = pod.namespace.as_deref()?;
        Some(Self::pod(namespace, pod.name.as_ref()))
    }

    /// Names the container to read.
    #[must_use]
    pub fn container(mut self, container: impl Into<String>) -> Self {
        self.container = Some(container.into());
        self
    }
}

impl fmt::Display for LogTarget {
    /// `namespace/pod` or `namespace/pod/container`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.namespace, self.pod)?;
        if let Some(container) = &self.container {
            write!(f, "/{container}")?;
        }
        Ok(())
    }
}
