//! What a Metrics tab is about: the six kinds that have history graphs, and the object of one.

use oxikube_domain::ids::Gvk;

/// Most pods of a workload a graph draws (one line each). A workload with more shows the first
/// by name and says how many were left out.
pub const MAX_WORKLOAD_PODS: usize = 20;

/// The kinds whose detail panel has a Metrics tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SubjectKind {
    /// A Pod: one line per container.
    Pod,
    /// A Node.
    Node,
    /// A Deployment, StatefulSet, DaemonSet, ReplicaSet or ReplicationController: one line per
    /// pod it selects.
    Workload,
    /// A Namespace.
    Namespace,
    /// A PersistentVolumeClaim.
    Pvc,
    /// An Ingress.
    Ingress,
}

impl SubjectKind {
    /// The graphs kind `gvk` has, `None` for a kind without history graphs.
    pub fn of(gvk: &Gvk) -> Option<Self> {
        match (&*gvk.group, &*gvk.kind) {
            ("", "Pod") => Some(Self::Pod),
            ("", "Node") => Some(Self::Node),
            ("", "Namespace") => Some(Self::Namespace),
            ("", "PersistentVolumeClaim") => Some(Self::Pvc),
            ("", "ReplicationController") => Some(Self::Workload),
            ("apps", "Deployment" | "StatefulSet" | "DaemonSet" | "ReplicaSet") => {
                Some(Self::Workload)
            }
            ("networking.k8s.io", "Ingress") => Some(Self::Ingress),
            _ => None,
        }
    }

    /// Whether the object is in a namespace (and so the target needs one).
    pub const fn namespaced(self) -> bool {
        !matches!(self, Self::Node | Self::Namespace)
    }

    /// Whether the graphs need the names of the object's pods (a workload's).
    pub const fn needs_pods(self) -> bool {
        matches!(self, Self::Workload)
    }
}

/// One object to draw graphs for, with the names its queries filter on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistorySubject {
    /// A pod.
    Pod {
        /// Its namespace.
        namespace: String,
        /// Its name.
        name: String,
    },
    /// A node.
    Node {
        /// Its name.
        name: String,
    },
    /// The pods of a workload, by name.
    Workload {
        /// Their namespace.
        namespace: String,
        /// At most [`MAX_WORKLOAD_PODS`] pod names, sorted.
        pods: Vec<String>,
        /// How many pods the workload has in all.
        total: usize,
    },
    /// A namespace.
    Namespace {
        /// Its name.
        name: String,
    },
    /// A persistent volume claim.
    Pvc {
        /// Its namespace.
        namespace: String,
        /// Its name.
        name: String,
    },
    /// An ingress.
    Ingress {
        /// Its namespace.
        namespace: String,
        /// Its name.
        name: String,
    },
}

impl HistorySubject {
    /// The subject of an object of `kind` called `name` in `namespace`. `pods` are the names of
    /// a workload's pods (ignored for other kinds); they are sorted and capped here.
    ///
    /// `None` when the object has no namespace and its kind needs one.
    pub fn new(
        kind: SubjectKind,
        namespace: Option<&str>,
        name: &str,
        pods: impl IntoIterator<Item = String>,
    ) -> Option<Self> {
        let namespace = || namespace.filter(|n| !n.is_empty()).map(str::to_owned);
        let name = name.to_owned();
        Some(match kind {
            SubjectKind::Pod => Self::Pod {
                namespace: namespace()?,
                name,
            },
            SubjectKind::Node => Self::Node { name },
            SubjectKind::Namespace => Self::Namespace { name },
            SubjectKind::Pvc => Self::Pvc {
                namespace: namespace()?,
                name,
            },
            SubjectKind::Ingress => Self::Ingress {
                namespace: namespace()?,
                name,
            },
            SubjectKind::Workload => {
                let mut pods: Vec<String> = pods.into_iter().collect();
                pods.sort();
                pods.dedup();
                let total = pods.len();
                pods.truncate(MAX_WORKLOAD_PODS);
                Self::Workload {
                    namespace: namespace()?,
                    pods,
                    total,
                }
            }
        })
    }

    /// The kind of the subject.
    pub fn kind(&self) -> SubjectKind {
        match self {
            Self::Pod { .. } => SubjectKind::Pod,
            Self::Node { .. } => SubjectKind::Node,
            Self::Workload { .. } => SubjectKind::Workload,
            Self::Namespace { .. } => SubjectKind::Namespace,
            Self::Pvc { .. } => SubjectKind::Pvc,
            Self::Ingress { .. } => SubjectKind::Ingress,
        }
    }

    /// Pods a workload has beyond the ones drawn (zero for other kinds).
    pub fn pods_left_out(&self) -> usize {
        match self {
            Self::Workload { pods, total, .. } => total.saturating_sub(pods.len()),
            _ => 0,
        }
    }

    /// Whether the subject has nothing to query yet (a workload with no pods).
    pub fn is_empty(&self) -> bool {
        matches!(self, Self::Workload { pods, .. } if pods.is_empty())
    }
}
