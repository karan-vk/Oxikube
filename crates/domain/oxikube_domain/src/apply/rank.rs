//! The kind-rank table: the one place that says which kinds apply before which.
//!
//! Lowest rank applies first. Adding a kind is one row. A kind that is not in the table (a custom
//! resource) ranks [`UNKNOWN_KIND_RANK`]: after every built-in except webhook configurations, so
//! it never comes before the CustomResourceDefinition that defines it. See the module docs of
//! [`crate::apply`] for why this is an approximation of kubectl.

use crate::ids::Gvk;

/// Rank of kinds the table does not list (custom resources): after built-ins and CRDs, before
/// admission webhooks.
pub const UNKNOWN_KIND_RANK: u8 = 12;

/// Rows of `(group, kind, rank)`; the core group is `""`.
const RANKS: &[(&str, &str, u8)] = &[
    ("", "Namespace", 0),
    ("", "ResourceQuota", 1),
    ("", "LimitRange", 1),
    ("scheduling.k8s.io", "PriorityClass", 1),
    ("networking.k8s.io", "NetworkPolicy", 2),
    ("extensions", "NetworkPolicy", 2),
    ("policy", "PodDisruptionBudget", 2),
    ("", "ServiceAccount", 3),
    ("", "Secret", 3),
    ("", "ConfigMap", 3),
    ("storage.k8s.io", "StorageClass", 4),
    ("", "PersistentVolume", 4),
    ("", "PersistentVolumeClaim", 4),
    ("apiextensions.k8s.io", "CustomResourceDefinition", 5),
    ("rbac.authorization.k8s.io", "ClusterRole", 6),
    ("rbac.authorization.k8s.io", "ClusterRoleBinding", 6),
    ("rbac.authorization.k8s.io", "Role", 6),
    ("rbac.authorization.k8s.io", "RoleBinding", 6),
    ("", "Service", 7),
    ("apps", "DaemonSet", 8),
    ("apps", "ReplicaSet", 8),
    ("apps", "Deployment", 8),
    ("apps", "StatefulSet", 8),
    ("extensions", "DaemonSet", 8),
    ("extensions", "ReplicaSet", 8),
    ("extensions", "Deployment", 8),
    ("", "ReplicationController", 8),
    ("autoscaling", "HorizontalPodAutoscaler", 9),
    ("batch", "Job", 10),
    ("batch", "CronJob", 10),
    ("", "Pod", 10),
    ("networking.k8s.io", "Ingress", 11),
    ("extensions", "Ingress", 11),
    ("networking.k8s.io", "IngressClass", 11),
    (
        "admissionregistration.k8s.io",
        "ValidatingWebhookConfiguration",
        13,
    ),
    (
        "admissionregistration.k8s.io",
        "MutatingWebhookConfiguration",
        13,
    ),
    (
        "admissionregistration.k8s.io",
        "ValidatingAdmissionPolicy",
        13,
    ),
    (
        "admissionregistration.k8s.io",
        "ValidatingAdmissionPolicyBinding",
        13,
    ),
];

/// The rank of `gvk`'s kind (version is ignored): lower applies first.
pub fn kind_rank(gvk: &Gvk) -> u8 {
    RANKS
        .iter()
        .find(|(group, kind, _)| *group == &*gvk.group && *kind == &*gvk.kind)
        .map_or(UNKNOWN_KIND_RANK, |(_, _, rank)| *rank)
}
