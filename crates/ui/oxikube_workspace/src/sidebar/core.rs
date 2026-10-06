//! The core sections of the sidebar, as placeholders (E06-S10).
//!
//! Lens-style groups: Cluster, Nodes, Workloads, Config, Network, Storage, Namespaces, Events,
//! Helm, Access Control, Custom Resources. Each entry names the kind it lists and the `list`
//! access it needs, which is all the visibility rules ask of it; the views behind the entries
//! arrive in E07 and later, which add entries and targets through
//! [`SidebarRegistry`](super::SidebarRegistry).

use gpui::App;
use oxikube_domain::access::AccessRequirement;
use oxikube_ui::IconName;

use super::{SidebarEntry, SidebarRegistry, SidebarSection, SidebarTarget};

/// Spacing between the core sections' `order`s, so a feature can slot a section between two.
pub const ORDER_STEP: u32 = 100;

fn kinds(entries: &[(&str, &str, &str, &str)]) -> Vec<SidebarEntry> {
    entries
        .iter()
        .map(|(id, title, group, resource)| SidebarEntry::kind(id, title, group, resource))
        .collect()
}

/// The core sections, in order.
pub fn core_sections() -> Vec<SidebarSection> {
    let section = |ix: u32, id: &str, title: &str, icon: IconName| {
        SidebarSection::new(id, title, icon, ix * ORDER_STEP)
    };
    vec![
        // Needs nothing: the overview is always reachable.
        section(1, "cluster", "Cluster", IconName::LayoutDashboard)
            .with_target(SidebarTarget::Page("overview".into())),
        section(2, "nodes", "Nodes", IconName::Server)
            .with_entries(kinds(&[("nodes", "Nodes", "", "nodes")])),
        section(3, "workloads", "Workloads", IconName::Boxes).with_entries(kinds(&[
            ("pods", "Pods", "", "pods"),
            ("deployments", "Deployments", "apps", "deployments"),
            ("daemonsets", "DaemonSets", "apps", "daemonsets"),
            ("statefulsets", "StatefulSets", "apps", "statefulsets"),
            ("replicasets", "ReplicaSets", "apps", "replicasets"),
            ("jobs", "Jobs", "batch", "jobs"),
            ("cronjobs", "CronJobs", "batch", "cronjobs"),
        ])),
        section(4, "config", "Config", IconName::FileText).with_entries(kinds(&[
            ("configmaps", "ConfigMaps", "", "configmaps"),
            ("secrets", "Secrets", "", "secrets"),
            ("resourcequotas", "Resource Quotas", "", "resourcequotas"),
            ("limitranges", "Limit Ranges", "", "limitranges"),
            (
                "hpas",
                "Horizontal Pod Autoscalers",
                "autoscaling",
                "horizontalpodautoscalers",
            ),
            (
                "pdbs",
                "Pod Disruption Budgets",
                "policy",
                "poddisruptionbudgets",
            ),
            (
                "priorityclasses",
                "Priority Classes",
                "scheduling.k8s.io",
                "priorityclasses",
            ),
            (
                "runtimeclasses",
                "Runtime Classes",
                "node.k8s.io",
                "runtimeclasses",
            ),
            ("leases", "Leases", "coordination.k8s.io", "leases"),
        ])),
        section(5, "network", "Network", IconName::Network).with_entries(kinds(&[
            ("services", "Services", "", "services"),
            ("endpoints", "Endpoints", "", "endpoints"),
            ("ingresses", "Ingresses", "networking.k8s.io", "ingresses"),
            (
                "ingressclasses",
                "Ingress Classes",
                "networking.k8s.io",
                "ingressclasses",
            ),
            (
                "networkpolicies",
                "Network Policies",
                "networking.k8s.io",
                "networkpolicies",
            ),
        ])),
        section(6, "storage", "Storage", IconName::HardDrive).with_entries(kinds(&[
            (
                "persistentvolumeclaims",
                "Persistent Volume Claims",
                "",
                "persistentvolumeclaims",
            ),
            (
                "persistentvolumes",
                "Persistent Volumes",
                "",
                "persistentvolumes",
            ),
            (
                "storageclasses",
                "Storage Classes",
                "storage.k8s.io",
                "storageclasses",
            ),
        ])),
        section(7, "namespaces", "Namespaces", IconName::Layers).with_entries(kinds(&[(
            "namespaces",
            "Namespaces",
            "",
            "namespaces",
        )])),
        // `events` exists in the core group and in `events.k8s.io`; either is enough.
        section(8, "events", "Events", IconName::Bell)
            .with_requires([
                AccessRequirement::list("", "events"),
                AccessRequirement::list("events.k8s.io", "events"),
            ])
            .with_target(SidebarTarget::kind("", "events")),
        // Helm 3 keeps its releases in Secrets (or ConfigMaps, depending on the storage driver).
        section(9, "helm", "Helm", IconName::Package).with_entries([SidebarEntry {
            requires: vec![
                AccessRequirement::list("", "secrets"),
                AccessRequirement::list("", "configmaps"),
            ],
            target: Some(SidebarTarget::Page("helm-releases".into())),
            ..SidebarEntry::kind("releases", "Releases", "", "secrets")
        }]),
        section(
            10,
            "access-control",
            "Access Control",
            IconName::ShieldCheck,
        )
        .with_entries(kinds(&[
            ("serviceaccounts", "Service Accounts", "", "serviceaccounts"),
            (
                "clusterroles",
                "Cluster Roles",
                "rbac.authorization.k8s.io",
                "clusterroles",
            ),
            ("roles", "Roles", "rbac.authorization.k8s.io", "roles"),
            (
                "clusterrolebindings",
                "Cluster Role Bindings",
                "rbac.authorization.k8s.io",
                "clusterrolebindings",
            ),
            (
                "rolebindings",
                "Role Bindings",
                "rbac.authorization.k8s.io",
                "rolebindings",
            ),
        ])),
        section(11, "custom-resources", "Custom Resources", IconName::Plug).custom_resources(),
    ]
}

/// Registers the core sections. Idempotent: registering them again replaces them.
pub fn register_core_sections(cx: &mut App) {
    for section in core_sections() {
        SidebarRegistry::register(cx, section);
    }
}
