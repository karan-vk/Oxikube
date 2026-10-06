//! [`CountTarget`]: a kind to count, and the table of built-in kinds the sidebar can badge.

use oxikube_domain::ids::{Gvk, Scope};

/// A kind to count: its identity and whether it is namespaced (which decides how the session's
/// namespace selection applies, [`WatchScope::derive`](oxikube_domain::session::WatchScope::derive)).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CountTarget {
    /// The kind.
    pub gvk: Gvk,
    /// Namespaced or cluster-scoped.
    pub scope: Scope,
}

impl CountTarget {
    /// A target for `gvk`.
    pub fn new(gvk: Gvk, scope: Scope) -> Self {
        Self { gvk, scope }
    }

    /// The built-in kind served at `plural` of API `group` (empty for core), as the sidebar names
    /// kinds. `None` for custom resources and anything not in [`CORE_TARGETS`]: they have no
    /// badge until a table opens their feed.
    pub fn core(group: &str, plural: &str) -> Option<Self> {
        CORE_TARGETS
            .iter()
            .find(|row| row.group == group && row.plural == plural)
            .map(CoreTarget::target)
    }
}

/// One row of [`CORE_TARGETS`].
#[derive(Debug, Clone, Copy)]
pub struct CoreTarget {
    /// API group (empty for core).
    pub group: &'static str,
    /// Plural resource name (`deployments`).
    pub plural: &'static str,
    /// API version the store watches.
    pub version: &'static str,
    /// Kind (`Deployment`).
    pub kind: &'static str,
    /// Whether the kind is namespaced.
    pub namespaced: bool,
}

impl CoreTarget {
    /// This row as a [`CountTarget`].
    pub fn target(&self) -> CountTarget {
        CountTarget::new(
            Gvk::new(self.group, self.version, self.kind),
            Scope::from_namespaced(self.namespaced),
        )
    }
}

const fn row(
    group: &'static str,
    plural: &'static str,
    version: &'static str,
    kind: &'static str,
    namespaced: bool,
) -> CoreTarget {
    CoreTarget {
        group,
        plural,
        version,
        kind,
        namespaced,
    }
}

/// The built-in kinds of the sidebar's core sections.
pub const CORE_TARGETS: &[CoreTarget] = &[
    row("", "nodes", "v1", "Node", false),
    row("", "namespaces", "v1", "Namespace", false),
    row("", "pods", "v1", "Pod", true),
    row("apps", "deployments", "v1", "Deployment", true),
    row("apps", "daemonsets", "v1", "DaemonSet", true),
    row("apps", "statefulsets", "v1", "StatefulSet", true),
    row("apps", "replicasets", "v1", "ReplicaSet", true),
    row("batch", "jobs", "v1", "Job", true),
    row("batch", "cronjobs", "v1", "CronJob", true),
    row("", "configmaps", "v1", "ConfigMap", true),
    row("", "secrets", "v1", "Secret", true),
    row("", "resourcequotas", "v1", "ResourceQuota", true),
    row("", "limitranges", "v1", "LimitRange", true),
    row(
        "autoscaling",
        "horizontalpodautoscalers",
        "v2",
        "HorizontalPodAutoscaler",
        true,
    ),
    row(
        "policy",
        "poddisruptionbudgets",
        "v1",
        "PodDisruptionBudget",
        true,
    ),
    row(
        "scheduling.k8s.io",
        "priorityclasses",
        "v1",
        "PriorityClass",
        false,
    ),
    row("node.k8s.io", "runtimeclasses", "v1", "RuntimeClass", false),
    row("coordination.k8s.io", "leases", "v1", "Lease", true),
    row("", "services", "v1", "Service", true),
    row("", "endpoints", "v1", "Endpoints", true),
    row("networking.k8s.io", "ingresses", "v1", "Ingress", true),
    row(
        "networking.k8s.io",
        "ingressclasses",
        "v1",
        "IngressClass",
        false,
    ),
    row(
        "networking.k8s.io",
        "networkpolicies",
        "v1",
        "NetworkPolicy",
        true,
    ),
    row(
        "",
        "persistentvolumeclaims",
        "v1",
        "PersistentVolumeClaim",
        true,
    ),
    row("", "persistentvolumes", "v1", "PersistentVolume", false),
    row(
        "storage.k8s.io",
        "storageclasses",
        "v1",
        "StorageClass",
        false,
    ),
    row("", "events", "v1", "Event", true),
    row("", "serviceaccounts", "v1", "ServiceAccount", true),
    row(
        "rbac.authorization.k8s.io",
        "clusterroles",
        "v1",
        "ClusterRole",
        false,
    ),
    row("rbac.authorization.k8s.io", "roles", "v1", "Role", true),
    row(
        "rbac.authorization.k8s.io",
        "clusterrolebindings",
        "v1",
        "ClusterRoleBinding",
        false,
    ),
    row(
        "rbac.authorization.k8s.io",
        "rolebindings",
        "v1",
        "RoleBinding",
        true,
    ),
];

/// The plurals of the kinds the Workloads overview counts, in tile order.
pub const WORKLOAD_TARGETS: [(&str, &str); 7] = [
    ("apps", "deployments"),
    ("apps", "statefulsets"),
    ("apps", "daemonsets"),
    ("apps", "replicasets"),
    ("batch", "jobs"),
    ("batch", "cronjobs"),
    ("", "pods"),
];
