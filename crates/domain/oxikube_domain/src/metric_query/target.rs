//! [`MetricTarget`]: the object a [`MetricRequest`](super::MetricRequest) is about.

use super::kind::MetricScope;

/// Which pods a [`MetricTarget::Pod`] covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PodSelector {
    /// One pod by exact name.
    One(String),
    /// Several pods by exact name (the pods a Deployment or StatefulSet owns right now).
    /// Matched as an anchored alternation of escaped names, never a prefix scan.
    Any(Vec<String>),
}

impl PodSelector {
    /// The names, in order.
    pub fn names(&self) -> &[String] {
        match self {
            PodSelector::One(name) => std::slice::from_ref(name),
            PodSelector::Any(names) => names,
        }
    }
}

/// The object a graph is about, with the names the queries filter on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetricTarget {
    /// The whole cluster.
    Cluster,
    /// A node, by its Kubernetes name.
    Node(String),
    /// A namespace.
    Namespace(String),
    /// Pods of one namespace; the result has one series per pod.
    Pod {
        /// The namespace.
        namespace: String,
        /// Which pods.
        pods: PodSelector,
    },
    /// The containers of one pod; the result has one series per container.
    Container {
        /// The namespace.
        namespace: String,
        /// The pod.
        pod: String,
        /// One container, or `None` for every container of the pod.
        container: Option<String>,
    },
    /// A PersistentVolumeClaim.
    Pvc {
        /// The namespace.
        namespace: String,
        /// The claim's name.
        name: String,
    },
    /// An Ingress.
    Ingress {
        /// The namespace.
        namespace: String,
        /// The Ingress's name.
        name: String,
    },
}

impl MetricTarget {
    /// The scope of this target.
    pub const fn scope(&self) -> MetricScope {
        match self {
            MetricTarget::Cluster => MetricScope::Cluster,
            MetricTarget::Node(_) => MetricScope::Node,
            MetricTarget::Namespace(_) => MetricScope::Namespace,
            MetricTarget::Pod { .. } => MetricScope::Pod,
            MetricTarget::Container { .. } => MetricScope::Container,
            MetricTarget::Pvc { .. } => MetricScope::Pvc,
            MetricTarget::Ingress { .. } => MetricScope::Ingress,
        }
    }

    /// The label that tells the series of a result apart, when it has more than one: `pod` for
    /// a pod target, `container` for a container target, none otherwise (one series).
    pub const fn series_label(&self) -> Option<&'static str> {
        match self {
            MetricTarget::Pod { .. } => Some("pod"),
            MetricTarget::Container { .. } => Some("container"),
            _ => None,
        }
    }
}
