//! RBAC, autoscaling and disruption kinds.
//!
//! Sources: Freelens Role / ClusterRole / RoleBinding / ClusterRoleBinding / HPA /
//! PodDisruptionBudget lists (A1); k9s `ro`, `rob`, `cr`, `crb`, `hpa`, `pdb` (A2); kubectl
//! printers.

use super::super::def::{
    AGE, ColumnDef, KindDef, LABELS, NAME, NAMESPACE,
    Src::{Func, Int, Text},
};
use super::super::funcs as f;

const ROLE: &[ColumnDef] = &[NAME, NAMESPACE, AGE, LABELS];
const CLUSTER_ROLE: &[ColumnDef] = &[NAME, AGE, LABELS];

const ROLE_BINDING: &[ColumnDef] = &[
    NAME,
    NAMESPACE,
    ColumnDef::new("role", "Role", Func(f::binding_role)),
    ColumnDef::new("subjects", "Subjects", Func(f::binding_subjects)),
    ColumnDef::new("subject-kinds", "Subject Kinds", Func(f::binding_kinds)),
    AGE,
    LABELS,
];

const CLUSTER_ROLE_BINDING: &[ColumnDef] = &[
    NAME,
    ColumnDef::new("role", "Role", Func(f::binding_role)),
    ColumnDef::new("subjects", "Subjects", Func(f::binding_subjects)),
    ColumnDef::new("subject-kinds", "Subject Kinds", Func(f::binding_kinds)),
    AGE,
    LABELS,
];

const HPA: &[ColumnDef] = &[
    NAME,
    NAMESPACE,
    ColumnDef::new("reference", "Reference", Func(f::hpa_reference)),
    ColumnDef::new("targets", "Targets", Func(f::hpa_targets)),
    ColumnDef::new("min-pods", "Min Pods", Int("/spec/minReplicas")).number(),
    ColumnDef::new("max-pods", "Max Pods", Int("/spec/maxReplicas")).number(),
    ColumnDef::new("replicas", "Replicas", Int("/status/currentReplicas")).number(),
    AGE,
    LABELS,
];

const PDB: &[ColumnDef] = &[
    NAME,
    NAMESPACE,
    ColumnDef::new("min-available", "Min Available", Text("/spec/minAvailable")).number(),
    ColumnDef::new(
        "max-unavailable",
        "Max Unavailable",
        Text("/spec/maxUnavailable"),
    )
    .number(),
    ColumnDef::new(
        "allowed-disruptions",
        "Allowed Disruptions",
        Int("/status/disruptionsAllowed"),
    )
    .number(),
    ColumnDef::new(
        "current-healthy",
        "Current Healthy",
        Int("/status/currentHealthy"),
    )
    .number(),
    ColumnDef::new(
        "desired-healthy",
        "Desired Healthy",
        Int("/status/desiredHealthy"),
    )
    .number(),
    AGE,
    LABELS,
];

pub(super) const KINDS: &[KindDef] = &[
    KindDef {
        group: "rbac.authorization.k8s.io",
        kind: "Role",
        columns: ROLE,
    },
    KindDef {
        group: "rbac.authorization.k8s.io",
        kind: "RoleBinding",
        columns: ROLE_BINDING,
    },
    KindDef {
        group: "rbac.authorization.k8s.io",
        kind: "ClusterRole",
        columns: CLUSTER_ROLE,
    },
    KindDef {
        group: "rbac.authorization.k8s.io",
        kind: "ClusterRoleBinding",
        columns: CLUSTER_ROLE_BINDING,
    },
    KindDef {
        group: "autoscaling",
        kind: "HorizontalPodAutoscaler",
        columns: HPA,
    },
    KindDef {
        group: "policy",
        kind: "PodDisruptionBudget",
        columns: PDB,
    },
];
