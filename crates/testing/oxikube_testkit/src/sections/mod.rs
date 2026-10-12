//! The section matrix (E12-S20): every kind the cluster sidebar lists, as the tests that guard
//! the per-kind work see it, with a manifest to seed one object of it.
//!
//! One [`KindCase`] per kind of the Freelens catalog (`docs/research/features-freelens-k9s.md`,
//! section A1) that the built-in sections register, plus the CustomResourceDefinition list that
//! the Custom Resources section opens. Each case states what its kind story shipped:
//!
//! * where it is reachable: the sidebar section and entry (`<section>/<plural>`);
//! * its **columns**: every column the kind's table offers, in order, a wide one (shown with
//!   "wide" only) suffixed `*`;
//! * its **actions**: the row actions of the kind beyond the generic ones every kind has (YAML,
//!   describe, edit, delete), in menu order;
//! * whether its detail has a panel of its own above the generic Overview;
//! * a **seed**: a manifest that a real API server accepts, so the kind suite creates one object of
//!   every kind (`seed`), and the fake suites insert the same object ([`seed_resource`]).
//!
//! The suites built on it: `bins/oxikube` `mount::tests::sections` (the real init order over
//! fakes: the registry, the sidebar, the tables, the detail drawer, the row actions and the guard
//! matrix) and `bins/oxikube/tests/kind_sections.rs` (the same against a kind cluster). A kind
//! that is registered but missing here, or a column or an action that changed, fails both: update
//! the case in the same PR as the change, which makes the change visible to the reviewer.
//!
//! | File | Holds |
//! |---|---|
//! | `cluster` | Nodes, Namespaces and the CustomResourceDefinition list |
//! | `workloads` | Pods, the replica controllers, DaemonSets, Jobs, CronJobs |
//! | `config` | ConfigMaps, Secrets, quotas, limits, autoscalers, budgets, classes, leases, admission |
//! | `network` | Services, Endpoints, EndpointSlices, Ingresses, IngressClasses, NetworkPolicies, the Gateway API |
//! | `storage` | claims, volumes, classes and the CSI snapshot kinds |
//!
//! Seeds are safe on a cluster other suites share: namespaced objects go to the test's own
//! namespace, cluster-scoped ones carry the test's unique stem in their name and do nothing
//! (webhook configurations without webhooks, an admission policy whose binding selects only the
//! test's namespace, a class nobody provisions for, a volume nothing claims). No seed schedules a
//! pod on a node it does not own: workloads have no replicas or select no node, the Job is
//! suspended and the CronJob suspended with a yearly schedule; the one Pod runs the pause image.

mod cluster;
mod config;
mod network;
mod storage;
mod workloads;

use oxikube_domain::Resource;
use oxikube_domain::command::CommandId;
use oxikube_domain::ids::{Gvk, Scope};
use serde_json::Value;

/// Where a kind's API comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Api {
    /// Every current API server serves it.
    BuiltIn,
    /// Only a cluster with the add-on's CRDs serves it (the Gateway API, the
    /// VerticalPodAutoscaler, the CSI snapshot kinds): the sidebar shows it only there, and the
    /// kind suite skips it, saying so, where discovery does not list it.
    Optional,
}

/// What a test seeds into: the namespace namespaced objects go to and the stem that makes the
/// names of cluster-scoped objects unique (the test namespace's own name is a good stem).
#[derive(Clone, Copy, Debug)]
pub struct Seed<'a> {
    /// The test's namespace.
    pub namespace: &'a str,
    /// A unique prefix for cluster-scoped names.
    pub stem: &'a str,
}

/// One kind of the section matrix. See the [module docs](self).
#[derive(Clone, Copy, Debug)]
pub struct KindCase {
    /// The sidebar section's id (`workloads`).
    pub section: &'static str,
    /// The kind's plural, also its sidebar entry's last part (`deployments`).
    pub plural: &'static str,
    /// API group (`""` for core).
    pub group: &'static str,
    /// The version the registry names.
    pub version: &'static str,
    /// The Kind.
    pub kind: &'static str,
    /// Namespaced or cluster-scoped.
    pub scope: Scope,
    /// Built in, or served only with an add-on's CRDs.
    pub api: Api,
    /// Every column the table offers, in order; wide ones end in `*`.
    pub columns: &'static [&'static str],
    /// The kind's own row actions, in menu order (the generic ones left out).
    pub actions: &'static [CommandId],
    /// Whether the detail has a panel of the kind's own.
    pub panel: bool,
    /// The manifest of the object a test seeds.
    pub(crate) seed: fn(&Seed<'_>) -> Value,
}

impl KindCase {
    /// The kind's group, version and Kind.
    pub fn gvk(&self) -> Gvk {
        Gvk::new(self.group, self.version, self.kind)
    }

    /// Whether the kind is namespaced.
    pub fn namespaced(&self) -> bool {
        self.scope == Scope::Namespaced
    }

    /// The sidebar entry that opens the kind's table (`workloads/deployments`).
    pub fn entry(&self) -> String {
        format!("{}/{}", self.section, self.plural)
    }

    /// The name of the object [`seed`](Self::seed) makes: `seed-<kind>` in the test's namespace,
    /// `<stem>-<kind>` for a cluster-scoped kind (the Namespace case is the test's namespace, and a
    /// CRD is named `<plural>.<group>` as the API server requires).
    pub fn seed_name(&self, seed: &Seed<'_>) -> String {
        let kind = self.kind.to_ascii_lowercase();
        match (self.kind, self.scope) {
            ("Namespace", _) => seed.namespace.to_owned(),
            ("CustomResourceDefinition", _) => cluster::definition_name(seed),
            (_, Scope::Namespaced) => format!("seed-{kind}"),
            (_, Scope::Cluster) => format!("{}-{kind}", seed.stem),
        }
    }

    /// The seeded object's manifest: `apiVersion`, `kind`, `metadata` (name, the namespace when
    /// namespaced, a `oxikube.test/suite: sections` label) and the kind's own fields.
    pub fn seed(&self, seed: &Seed<'_>) -> Value {
        let mut manifest = (self.seed)(seed);
        let api_version = if self.group.is_empty() {
            self.version.to_owned()
        } else {
            format!("{}/{}", self.group, self.version)
        };
        manifest["apiVersion"] = Value::from(api_version);
        manifest["kind"] = Value::from(self.kind);
        let metadata = &mut manifest["metadata"];
        metadata["name"] = Value::from(self.seed_name(seed));
        if self.namespaced() {
            metadata["namespace"] = Value::from(seed.namespace);
        }
        metadata["labels"][SUITE_LABEL] = Value::from("sections");
        manifest
    }

    /// Whether the kind suite creates the seed on the cluster: everything but the Node (the
    /// cluster's own node is read, never one made up) and the Namespace (the test's own).
    pub fn seeded_on_cluster(&self) -> bool {
        !matches!(self.kind, "Node" | "Namespace")
    }
}

/// The label every seeded object carries.
pub const SUITE_LABEL: &str = "oxikube.test/suite";

/// The sidebar sections in the order the sidebar lists them, as the matrix knows them.
pub const SECTIONS: &[&str] = &[
    "nodes",
    "workloads",
    "config",
    "network",
    "storage",
    "namespaces",
    "custom-resources",
];

/// Every case, section by section in sidebar order, each section's kinds in their sidebar order.
pub fn catalog() -> impl Iterator<Item = &'static KindCase> {
    cluster::NODES
        .iter()
        .chain(workloads::CASES)
        .chain(config::CASES)
        .chain(network::CASES)
        .chain(storage::CASES)
        .chain(cluster::NAMESPACES)
        .chain(cluster::DEFINITIONS)
}

/// The cases of `section`, in sidebar order.
pub fn in_section(section: &str) -> impl Iterator<Item = &'static KindCase> + '_ {
    catalog().filter(move |case| case.section == section)
}

/// The case of `kind` (`Deployment`), if the matrix has it.
pub fn case(kind: &str) -> Option<&'static KindCase> {
    catalog().find(|case| case.kind == kind)
}

/// `case`'s seed as a domain [`Resource`], with what an API server adds (a uid, a resource
/// version and a creation time), for the fake suites.
///
/// # Panics
///
/// When the seed is not an object with a name (a bug in the catalog).
pub fn seed_resource(case: &KindCase, seed: &Seed<'_>) -> Resource {
    let mut manifest = case.seed(seed);
    let metadata = &mut manifest["metadata"];
    metadata["uid"] = Value::from(format!("uid-{}", case.seed_name(seed)));
    metadata["resourceVersion"] = Value::from("1");
    metadata["creationTimestamp"] = Value::from("2026-01-01T00:00:00Z");
    Resource::from_json(manifest).expect("a seed is an object with a name")
}

#[cfg(test)]
mod tests;
