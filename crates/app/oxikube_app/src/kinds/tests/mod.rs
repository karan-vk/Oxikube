//! Tests of the kind registry: registration, lookup and fallback (`registry`), discovery and
//! visibility gating (`gate`), the sidebar tree's order and search (`tree`). Pure: no cluster,
//! no window.

mod gate;
mod registry;
mod tree;

use oxikube_domain::ids::{Gvk, Scope};

use super::{KindMeta, Section};

/// A namespaced kind in `section` at `rank`.
pub(super) fn meta(group: &str, kind: &str, plural: &str, section: Section, rank: u32) -> KindMeta {
    KindMeta::new(
        Gvk::new(group, "v1", kind),
        plural,
        &format!("{kind}s"),
        Scope::Namespaced,
    )
    .in_section(section, rank)
}

/// A registry the shape of the built-in one: a few kinds in each of several sections.
pub(super) fn built_ins() -> Vec<KindMeta> {
    vec![
        KindMeta::new(Gvk::new("", "v1", "Node"), "nodes", "Nodes", Scope::Cluster)
            .in_section(Section::NODES, 0),
        meta("", "Pod", "pods", Section::WORKLOADS, 10),
        meta("apps", "Deployment", "deployments", Section::WORKLOADS, 20),
        meta(
            "apps",
            "StatefulSet",
            "statefulsets",
            Section::WORKLOADS,
            40,
        ),
        meta("apps", "DaemonSet", "daemonsets", Section::WORKLOADS, 30),
        meta("", "ConfigMap", "configmaps", Section::CONFIG, 10),
        meta("", "Secret", "secrets", Section::CONFIG, 20),
        meta("", "Service", "services", Section::NETWORK, 10),
        meta(
            "gateway.networking.k8s.io",
            "Gateway",
            "gateways",
            Section::NETWORK,
            80,
        ),
        KindMeta::new(
            Gvk::new("", "v1", "Namespace"),
            "namespaces",
            "Namespaces",
            Scope::Cluster,
        )
        .in_section(Section::NAMESPACES, 0),
    ]
}
