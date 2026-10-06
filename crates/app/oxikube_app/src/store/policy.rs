//! [`FeedPolicy`]: which feed serves a kind (ADR 0006), as a table later stories can extend.
//!
//! Core kinds get a reflector feed ([`FeedKind::Full`]) so views can compute their own columns
//! (restarts, readiness); bulky or sensitive core kinds get a metadata-only feed
//! ([`FeedKind::Metadata`]: Secrets never hold decoded data in the cache, non-negotiable 5);
//! CRDs and every kind not in the table get the server-side Table feed ([`FeedKind::Table`]), so
//! they render with their `additionalPrinterColumns`. The policy answers with a [`FeedPlan`],
//! which [`open`](super::feed::open) turns into a port call; nothing here names a kube type.

use std::collections::HashMap;

use oxikube_domain::ids::Gvk;
use oxikube_ports::FeedVariant;

/// Which port feed carries a kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FeedKind {
    /// `ResourceReader::watch`: whole objects.
    Full,
    /// `ResourceReader::watch` with `metadata_only`: partial objects.
    Metadata,
    /// `TableFeedPort::table_feed`: server-side Table rows.
    Table,
}

impl FeedKind {
    /// The watch-budget counter variant this kind maps to.
    pub fn variant(self) -> FeedVariant {
        match self {
            FeedKind::Full => FeedVariant::Full,
            FeedKind::Metadata => FeedVariant::Metadata,
            FeedKind::Table => FeedVariant::Table,
        }
    }
}

/// How much a kind's feed matters when the [`FeedBudget`](super::FeedBudget) is tight.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FeedPriority {
    /// Background kinds (CRDs, rarely opened views).
    Low,
    /// Ordinary kinds. The default.
    #[default]
    Normal,
    /// Kinds the overview and sidebar counts always need (pods, nodes, namespaces, deployments).
    High,
}

/// The policy's answer for one kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FeedPlan {
    /// Which feed to open.
    pub kind: FeedKind,
    /// Its budget priority.
    pub priority: FeedPriority,
}

impl FeedPlan {
    /// A plan with the given feed and priority.
    pub const fn new(kind: FeedKind, priority: FeedPriority) -> Self {
        Self { kind, priority }
    }
}

use FeedKind::{Full, Metadata};
use FeedPriority::{High, Normal};

/// The built-in table: (group, kind) → plan. Versions are ignored, so `v1` and `v1beta1` of a
/// kind share a plan.
const CORE: &[(&str, &str, FeedPlan)] = &[
    ("", "Pod", FeedPlan::new(Full, High)),
    ("", "Node", FeedPlan::new(Full, High)),
    ("", "Namespace", FeedPlan::new(Full, High)),
    ("", "Service", FeedPlan::new(Full, Normal)),
    ("", "Endpoints", FeedPlan::new(Full, Normal)),
    ("", "Event", FeedPlan::new(Full, Normal)),
    ("", "PersistentVolume", FeedPlan::new(Full, Normal)),
    ("", "PersistentVolumeClaim", FeedPlan::new(Full, Normal)),
    ("", "ServiceAccount", FeedPlan::new(Full, Normal)),
    ("", "ResourceQuota", FeedPlan::new(Full, Normal)),
    ("", "LimitRange", FeedPlan::new(Full, Normal)),
    ("", "ReplicationController", FeedPlan::new(Full, Normal)),
    ("", "ConfigMap", FeedPlan::new(Metadata, Normal)),
    ("", "Secret", FeedPlan::new(Metadata, Normal)),
    ("apps", "Deployment", FeedPlan::new(Full, High)),
    ("apps", "ReplicaSet", FeedPlan::new(Full, Normal)),
    ("apps", "StatefulSet", FeedPlan::new(Full, Normal)),
    ("apps", "DaemonSet", FeedPlan::new(Full, Normal)),
    ("batch", "Job", FeedPlan::new(Full, Normal)),
    ("batch", "CronJob", FeedPlan::new(Full, Normal)),
    ("networking.k8s.io", "Ingress", FeedPlan::new(Full, Normal)),
    (
        "networking.k8s.io",
        "IngressClass",
        FeedPlan::new(Full, Normal),
    ),
    (
        "networking.k8s.io",
        "NetworkPolicy",
        FeedPlan::new(Full, Normal),
    ),
    (
        "discovery.k8s.io",
        "EndpointSlice",
        FeedPlan::new(Full, Normal),
    ),
    (
        "storage.k8s.io",
        "StorageClass",
        FeedPlan::new(Full, Normal),
    ),
    (
        "rbac.authorization.k8s.io",
        "Role",
        FeedPlan::new(Full, Normal),
    ),
    (
        "rbac.authorization.k8s.io",
        "RoleBinding",
        FeedPlan::new(Full, Normal),
    ),
    (
        "rbac.authorization.k8s.io",
        "ClusterRole",
        FeedPlan::new(Full, Normal),
    ),
    (
        "rbac.authorization.k8s.io",
        "ClusterRoleBinding",
        FeedPlan::new(Full, Normal),
    ),
    (
        "autoscaling",
        "HorizontalPodAutoscaler",
        FeedPlan::new(Full, Normal),
    ),
    ("policy", "PodDisruptionBudget", FeedPlan::new(Full, Normal)),
    (
        "coordination.k8s.io",
        "Lease",
        FeedPlan::new(Metadata, Normal),
    ),
];

/// The plan for kinds the table does not list: CRDs and unknown kinds read the Table API.
pub const FALLBACK: FeedPlan = FeedPlan::new(FeedKind::Table, FeedPriority::Low);

/// Table-driven feed choice: the built-in [`CORE`] rows plus per-kind overrides (E12 adds its
/// own; a user setting could too).
#[derive(Debug, Clone)]
pub struct FeedPolicy {
    rows: HashMap<(String, String), FeedPlan>,
    fallback: FeedPlan,
}

impl Default for FeedPolicy {
    fn default() -> Self {
        Self {
            rows: CORE
                .iter()
                .map(|(g, k, plan)| (((*g).to_owned(), (*k).to_owned()), *plan))
                .collect(),
            fallback: FALLBACK,
        }
    }
}

impl FeedPolicy {
    /// The built-in policy.
    pub fn new() -> Self {
        Self::default()
    }

    /// Overrides the plan of `group`/`kind` (empty group for core).
    #[must_use]
    pub fn with_override(mut self, group: &str, kind: &str, plan: FeedPlan) -> Self {
        self.rows.insert((group.to_owned(), kind.to_owned()), plan);
        self
    }

    /// Sets the plan for kinds the table does not list.
    #[must_use]
    pub fn with_fallback(mut self, plan: FeedPlan) -> Self {
        self.fallback = plan;
        self
    }

    /// The plan for `gvk`.
    pub fn plan(&self, gvk: &Gvk) -> FeedPlan {
        self.rows
            .get(&(gvk.group.to_string(), gvk.kind.to_string()))
            .copied()
            .unwrap_or(self.fallback)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_kinds_use_reflectors_and_crds_use_the_table_api() {
        let policy = FeedPolicy::new();
        assert_eq!(policy.plan(&Gvk::new("", "v1", "Pod")).kind, FeedKind::Full);
        assert_eq!(
            policy.plan(&Gvk::new("", "v1", "Pod")).priority,
            FeedPriority::High
        );
        assert_eq!(
            policy.plan(&Gvk::new("apps", "v1", "Deployment")).kind,
            FeedKind::Full
        );
        assert_eq!(
            policy.plan(&Gvk::new("", "v1", "Secret")).kind,
            FeedKind::Metadata
        );
        assert_eq!(
            policy.plan(&Gvk::new("example.com", "v1", "Widget")),
            FALLBACK
        );
        assert_eq!(FeedKind::Table.variant(), FeedVariant::Table);
    }

    #[test]
    fn overrides_win_and_ignore_the_version() {
        let policy = FeedPolicy::new()
            .with_override("", "Pod", FeedPlan::new(FeedKind::Table, FeedPriority::Low))
            .with_fallback(FeedPlan::new(FeedKind::Metadata, FeedPriority::Normal));
        assert_eq!(
            policy.plan(&Gvk::new("", "v2", "Pod")).kind,
            FeedKind::Table
        );
        assert_eq!(
            policy.plan(&Gvk::new("x.io", "v1", "Thing")).kind,
            FeedKind::Metadata
        );
    }
}
