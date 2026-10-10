//! Discovery fixtures: [`ResourceKind`]s as an API server reports them, for alias, picker and
//! sidebar tests.
//!
//! [`kind`] builds one served type (`kind("", "v1", "Pod", "pods").short("po").build()`);
//! [`core_kinds`] is a realistic set of the types every cluster serves; [`cert_manager_kinds`]
//! and [`clashing_crds`] are the CRD fixtures the alias table's collision rules are tested on:
//! two groups with the same plural (cert-manager's `Certificate` next to another vendor's), and
//! short names that equal built-in aliases.

use oxikube_domain::ids::Gvk;
use oxikube_domain::kinds::{ResourceKind, Verb, VerbSet};

/// What [`kind`] builds from.
#[derive(Debug, Clone)]
pub struct KindSpec {
    kind: ResourceKind,
}

/// A served type: group (`""` for core), version, Kind and plural. Preferred, namespaced, all
/// verbs, no singular and no short names until asked.
pub fn kind(group: &str, version: &str, kind: &str, plural: &str) -> KindSpec {
    KindSpec::new(group, version, kind, plural)
}

impl KindSpec {
    /// Same as [`kind`].
    pub fn new(group: &str, version: &str, kind: &str, plural: &str) -> Self {
        Self {
            kind: ResourceKind {
                gvk: Gvk::new(group, version, kind),
                preferred: true,
                plural: plural.to_owned(),
                singular: String::new(),
                short_names: Vec::new(),
                categories: Vec::new(),
                verbs: VerbSet::from_names([
                    "create",
                    "delete",
                    "deletecollection",
                    "get",
                    "list",
                    "patch",
                    "update",
                    "watch",
                ]),
                namespaced: true,
            },
        }
    }

    /// Adds a short name.
    #[must_use]
    pub fn short(mut self, name: &str) -> Self {
        self.kind.short_names.push(name.to_owned());
        self
    }

    /// Sets the singular name.
    #[must_use]
    pub fn singular(mut self, name: &str) -> Self {
        self.kind.singular = name.to_owned();
        self
    }

    /// Cluster-scoped instead of namespaced.
    #[must_use]
    pub fn cluster_scoped(mut self) -> Self {
        self.kind.namespaced = false;
        self
    }

    /// Not the server's preferred version of the type.
    #[must_use]
    pub fn not_preferred(mut self) -> Self {
        self.kind.preferred = false;
        self
    }

    /// Only listable (no watch).
    #[must_use]
    pub fn list_only(mut self) -> Self {
        self.kind.verbs = [Verb::Get, Verb::List].into();
        self
    }

    /// The finished record.
    pub fn build(self) -> ResourceKind {
        self.kind
    }
}

impl From<KindSpec> for ResourceKind {
    fn from(spec: KindSpec) -> Self {
        spec.build()
    }
}

/// The types of a stock cluster, as `kubectl api-resources` lists them: the short names are the
/// server's (`po`, `deploy`, `svc`, ...), not k9s's (`dp`, `sec`, ...).
pub fn core_kinds() -> Vec<ResourceKind> {
    let k = |group: &str, version: &str, kind_name: &str, plural: &str| {
        KindSpec::new(group, version, kind_name, plural).singular(&kind_name.to_ascii_lowercase())
    };
    vec![
        k("", "v1", "Pod", "pods").short("po").build(),
        k("", "v1", "Service", "services").short("svc").build(),
        k("", "v1", "ConfigMap", "configmaps").short("cm").build(),
        k("", "v1", "Secret", "secrets").build(),
        k("", "v1", "ServiceAccount", "serviceaccounts")
            .short("sa")
            .build(),
        k("", "v1", "Namespace", "namespaces")
            .short("ns")
            .cluster_scoped()
            .build(),
        k("", "v1", "Node", "nodes")
            .short("no")
            .cluster_scoped()
            .build(),
        k("", "v1", "Event", "events").short("ev").build(),
        k("", "v1", "Endpoints", "endpoints").short("ep").build(),
        k("", "v1", "PersistentVolume", "persistentvolumes")
            .short("pv")
            .cluster_scoped()
            .build(),
        k("", "v1", "PersistentVolumeClaim", "persistentvolumeclaims")
            .short("pvc")
            .build(),
        k("apps", "v1", "Deployment", "deployments")
            .short("deploy")
            .build(),
        k("apps", "v1", "StatefulSet", "statefulsets")
            .short("sts")
            .build(),
        k("apps", "v1", "DaemonSet", "daemonsets")
            .short("ds")
            .build(),
        k("apps", "v1", "ReplicaSet", "replicasets")
            .short("rs")
            .build(),
        k("batch", "v1", "Job", "jobs").build(),
        k("batch", "v1", "CronJob", "cronjobs").short("cj").build(),
        k("networking.k8s.io", "v1", "Ingress", "ingresses")
            .short("ing")
            .build(),
        k(
            "networking.k8s.io",
            "v1",
            "NetworkPolicy",
            "networkpolicies",
        )
        .short("netpol")
        .build(),
        k("storage.k8s.io", "v1", "StorageClass", "storageclasses")
            .short("sc")
            .cluster_scoped()
            .build(),
        k("rbac.authorization.k8s.io", "v1", "Role", "roles").build(),
        k(
            "rbac.authorization.k8s.io",
            "v1",
            "RoleBinding",
            "rolebindings",
        )
        .build(),
        k(
            "rbac.authorization.k8s.io",
            "v1",
            "ClusterRole",
            "clusterroles",
        )
        .cluster_scoped()
        .build(),
        k(
            "rbac.authorization.k8s.io",
            "v1",
            "ClusterRoleBinding",
            "clusterrolebindings",
        )
        .cluster_scoped()
        .build(),
        k(
            "autoscaling",
            "v2",
            "HorizontalPodAutoscaler",
            "horizontalpodautoscalers",
        )
        .short("hpa")
        .build(),
        k(
            "policy",
            "v1",
            "PodDisruptionBudget",
            "poddisruptionbudgets",
        )
        .short("pdb")
        .build(),
        k(
            "apiextensions.k8s.io",
            "v1",
            "CustomResourceDefinition",
            "customresourcedefinitions",
        )
        .short("crd")
        .short("crds")
        .cluster_scoped()
        .build(),
        // The events API: same plural and Kind as the core `Event`, and the same short name.
        k("events.k8s.io", "v1", "Event", "events")
            .short("ev")
            .build(),
    ]
}

/// cert-manager's types: `Certificate` (`cert`, `certs`), `Issuer`, `ClusterIssuer`.
pub fn cert_manager_kinds() -> Vec<ResourceKind> {
    let k =
        |kind_name: &str, plural: &str| KindSpec::new("cert-manager.io", "v1", kind_name, plural);
    vec![
        k("Certificate", "certificates")
            .singular("certificate")
            .short("cert")
            .short("certs")
            .build(),
        k("Issuer", "issuers").singular("issuer").build(),
        k("ClusterIssuer", "clusterissuers")
            .singular("clusterissuer")
            .cluster_scoped()
            .build(),
    ]
}

/// CRDs that collide with each other and with the built-in aliases:
///
/// - `Certificate` in `example.io` has the same plural as cert-manager's, and the short name
///   `cert`, so `certificates` and `cert` are ambiguous between the two groups;
/// - `Rollout` in `rollouts.io` has the short name `dp`, which the built-in table already gives
///   to Deployments;
/// - `Widget` has a short name `wg` nobody else uses (no collision).
pub fn clashing_crds() -> Vec<ResourceKind> {
    vec![
        KindSpec::new("example.io", "v1", "Certificate", "certificates")
            .singular("certificate")
            .short("cert")
            .build(),
        KindSpec::new("rollouts.io", "v1alpha1", "Rollout", "rollouts")
            .singular("rollout")
            .short("dp")
            .build(),
        KindSpec::new("example.io", "v1", "Widget", "widgets")
            .singular("widget")
            .short("wg")
            .build(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_core_set_is_unique_per_group_and_plural() {
        let kinds = core_kinds();
        let mut seen = std::collections::BTreeSet::new();
        for kind in &kinds {
            assert!(
                seen.insert((kind.gvk.group.clone(), kind.plural.clone())),
                "{} twice",
                kind
            );
        }
        assert!(kinds.iter().any(|k| k.gvk.is_pod() && k.matches_name("po")));
    }

    #[test]
    fn the_builder_sets_what_it_says() {
        let k = kind("g.io", "v1", "Thing", "things")
            .short("th")
            .singular("thing")
            .cluster_scoped()
            .not_preferred()
            .list_only()
            .build();
        assert_eq!(k.short_names, ["th"]);
        assert!(!k.namespaced && !k.preferred && !k.is_watchable());
    }
}
