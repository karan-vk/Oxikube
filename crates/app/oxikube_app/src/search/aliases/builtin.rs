//! The built-in aliases: k9s's short names plus the plural and singular of every core type, so
//! `:po`, `:pods` and `:pod` work before the cluster has answered discovery.
//!
//! This is data, not code: one row per type with every name it answers to. The version in a row
//! is only a default; when discovery serves the type the table uses the version the server
//! prefers ([`Discovered::served_version`](super::discovered::Discovered::served_version)), so
//! `hpa` follows `autoscaling/v2` on a new cluster and `v2beta2` on an old one.

use std::sync::Arc;

use oxikube_domain::AliasTarget;
use oxikube_domain::ids::Gvr;

use super::discovered::Discovered;
use super::entry::{AliasEntry, AliasSource};

/// One built-in type and the names it answers to.
pub(super) struct Row {
    /// Lower-case names: k9s short names first, then singular and plural.
    pub names: &'static [&'static str],
    /// API group (`""` for core).
    pub group: &'static str,
    /// Version to use when discovery has not said otherwise.
    pub version: &'static str,
    /// Plural resource name.
    pub resource: &'static str,
    /// The Kind (`Pod`), which a list is opened by.
    pub kind: &'static str,
}

const fn row(
    names: &'static [&'static str],
    group: &'static str,
    version: &'static str,
    resource: &'static str,
    kind: &'static str,
) -> Row {
    Row {
        names,
        group,
        version,
        resource,
        kind,
    }
}

/// The table. Names are unique across rows (a test checks it).
pub(super) static BUILTINS: &[Row] = &[
    // Core.
    row(&["po", "pod", "pods"], "", "v1", "pods", "Pod"),
    row(
        &["svc", "service", "services"],
        "",
        "v1",
        "services",
        "Service",
    ),
    row(
        &["cm", "configmap", "configmaps"],
        "",
        "v1",
        "configmaps",
        "ConfigMap",
    ),
    row(&["sec", "secret", "secrets"], "", "v1", "secrets", "Secret"),
    row(
        &["sa", "serviceaccount", "serviceaccounts"],
        "",
        "v1",
        "serviceaccounts",
        "ServiceAccount",
    ),
    row(
        &["ns", "namespace", "namespaces"],
        "",
        "v1",
        "namespaces",
        "Namespace",
    ),
    row(&["no", "node", "nodes"], "", "v1", "nodes", "Node"),
    row(&["ev", "event", "events"], "", "v1", "events", "Event"),
    row(&["ep", "endpoints"], "", "v1", "endpoints", "Endpoints"),
    row(
        &["pv", "persistentvolume", "persistentvolumes"],
        "",
        "v1",
        "persistentvolumes",
        "PersistentVolume",
    ),
    row(
        &["pvc", "persistentvolumeclaim", "persistentvolumeclaims"],
        "",
        "v1",
        "persistentvolumeclaims",
        "PersistentVolumeClaim",
    ),
    row(
        &["rc", "replicationcontroller", "replicationcontrollers"],
        "",
        "v1",
        "replicationcontrollers",
        "ReplicationController",
    ),
    row(
        &["rq", "quota", "resourcequota", "resourcequotas"],
        "",
        "v1",
        "resourcequotas",
        "ResourceQuota",
    ),
    row(
        &["limits", "limitrange", "limitranges"],
        "",
        "v1",
        "limitranges",
        "LimitRange",
    ),
    // Workloads.
    row(
        &["dp", "deploy", "deployment", "deployments"],
        "apps",
        "v1",
        "deployments",
        "Deployment",
    ),
    row(
        &["sts", "statefulset", "statefulsets"],
        "apps",
        "v1",
        "statefulsets",
        "StatefulSet",
    ),
    row(
        &["ds", "daemonset", "daemonsets"],
        "apps",
        "v1",
        "daemonsets",
        "DaemonSet",
    ),
    row(
        &["rs", "replicaset", "replicasets"],
        "apps",
        "v1",
        "replicasets",
        "ReplicaSet",
    ),
    row(&["job", "jobs"], "batch", "v1", "jobs", "Job"),
    row(
        &["cj", "cronjob", "cronjobs"],
        "batch",
        "v1",
        "cronjobs",
        "CronJob",
    ),
    // Networking and storage.
    row(
        &["ing", "ingress", "ingresses"],
        "networking.k8s.io",
        "v1",
        "ingresses",
        "Ingress",
    ),
    row(
        &["np", "netpol", "networkpolicy", "networkpolicies"],
        "networking.k8s.io",
        "v1",
        "networkpolicies",
        "NetworkPolicy",
    ),
    row(
        &["ingressclass", "ingressclasses"],
        "networking.k8s.io",
        "v1",
        "ingressclasses",
        "IngressClass",
    ),
    row(
        &["eps", "endpointslice", "endpointslices"],
        "discovery.k8s.io",
        "v1",
        "endpointslices",
        "EndpointSlice",
    ),
    row(
        &["sc", "storageclass", "storageclasses"],
        "storage.k8s.io",
        "v1",
        "storageclasses",
        "StorageClass",
    ),
    // RBAC.
    row(
        &["ro", "role", "roles"],
        "rbac.authorization.k8s.io",
        "v1",
        "roles",
        "Role",
    ),
    row(
        &["rb", "rob", "rolebinding", "rolebindings"],
        "rbac.authorization.k8s.io",
        "v1",
        "rolebindings",
        "RoleBinding",
    ),
    row(
        &["cr", "clusterrole", "clusterroles"],
        "rbac.authorization.k8s.io",
        "v1",
        "clusterroles",
        "ClusterRole",
    ),
    row(
        &["crb", "clusterrolebinding", "clusterrolebindings"],
        "rbac.authorization.k8s.io",
        "v1",
        "clusterrolebindings",
        "ClusterRoleBinding",
    ),
    // Policy, scaling, scheduling, extension points.
    row(
        &["hpa", "horizontalpodautoscaler", "horizontalpodautoscalers"],
        "autoscaling",
        "v2",
        "horizontalpodautoscalers",
        "HorizontalPodAutoscaler",
    ),
    row(
        &["pdb", "poddisruptionbudget", "poddisruptionbudgets"],
        "policy",
        "v1",
        "poddisruptionbudgets",
        "PodDisruptionBudget",
    ),
    row(
        &["pc", "priorityclass", "priorityclasses"],
        "scheduling.k8s.io",
        "v1",
        "priorityclasses",
        "PriorityClass",
    ),
    row(
        &[
            "crd",
            "crds",
            "customresourcedefinition",
            "customresourcedefinitions",
        ],
        "apiextensions.k8s.io",
        "v1",
        "customresourcedefinitions",
        "CustomResourceDefinition",
    ),
];

/// The built-in entries, each pointing at the version `discovered` serves for its type (the row's
/// default when the cluster does not serve it or has not answered).
pub(super) fn entries(discovered: &Discovered) -> Vec<AliasEntry> {
    let mut out = Vec::with_capacity(BUILTINS.iter().map(|r| r.names.len()).sum());
    for row in BUILTINS {
        let version = discovered
            .served_version(row.group, row.resource)
            .unwrap_or_else(|| Arc::from(row.version));
        let target = AliasTarget::Gvr(Gvr::new(row.group, version, row.resource));
        for name in row.names {
            out.push(AliasEntry {
                name: Arc::from(*name),
                target: target.clone(),
                source: AliasSource::BuiltIn,
            });
        }
    }
    out
}
