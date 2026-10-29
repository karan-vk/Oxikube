//! [`classify`]: the pure, table-driven risk of one mutation.
//!
//! The risk of a mutation is a function of the verb, whether the kind is namespaced, the kind
//! itself and a few flags. Everything that decides a risk is data in this file: the verb floor
//! ([`verb_floor`]) and the kind rows ([`KIND_RULES`]); adding a kind is one row.
//! [`crate::command::delete_risk`] is this function for [`MutationVerb::Delete`], so the guard,
//! the confirmation dialog and the MCP tool stubs cannot disagree.
//!
//! | Rule | Risk floor |
//! |---|---|
//! | create, update, patch, apply, scale, restart, cordon, uncordon | [`Risk::Low`] |
//! | delete, evict | [`Risk::Medium`] |
//! | drain, delete-collection, prune | [`Risk::High`] |
//! | `Namespace` or `PersistentVolume` delete or delete-collection | [`Risk::Irreversible`] |
//! | `Node` or `CustomResourceDefinition` delete | [`Risk::High`] |
//! | RBAC kind written | [`Risk::Medium`] |
//! | unknown (custom) kind written, or any cluster-scoped write | [`Risk::Medium`] |
//! | [`MutationFlags::cascading`] delete | [`Risk::High`] |
//! | [`MutationFlags::force`] delete, evict or drain | [`Risk::High`] |
//! | [`MutationFlags::to_zero`] scale | [`Risk::Medium`] |
//! | [`MutationFlags::all_namespaces`] | [`Risk::High`] for removing verbs, else [`Risk::Medium`] |
//!
//! A floor only raises; the result is the maximum of every row that applies.

use super::{MutationVerb, Risk};
use crate::ids::{Gvk, Scope};

/// Modifiers of a mutation that move its risk. All default to `false`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct MutationFlags {
    /// Dependents are deleted too (foreground propagation).
    pub cascading: bool,
    /// The graceful path is skipped (grace period 0, eviction override, drain `--force`).
    pub force: bool,
    /// The request spans every namespace.
    pub all_namespaces: bool,
    /// A scale to zero replicas.
    pub to_zero: bool,
}

/// A `(group, kind)` pair and the risk floor it sets for some verbs.
struct KindRule {
    group: &'static str,
    kind: &'static str,
    verbs: &'static [MutationVerb],
    floor: Risk,
}

const REMOVING: &[MutationVerb] = &[MutationVerb::Delete, MutationVerb::DeleteCollection];
const WRITING: &[MutationVerb] = &[
    MutationVerb::Create,
    MutationVerb::Update,
    MutationVerb::Patch,
    MutationVerb::Apply,
];
const RBAC_GROUP: &str = "rbac.authorization.k8s.io";

/// One row per kind that is riskier than its verb alone.
const KIND_RULES: &[KindRule] = &[
    KindRule {
        group: "",
        kind: "Namespace",
        verbs: REMOVING,
        floor: Risk::Irreversible,
    },
    KindRule {
        group: "",
        kind: "PersistentVolume",
        verbs: REMOVING,
        floor: Risk::Irreversible,
    },
    KindRule {
        group: "",
        kind: "Node",
        verbs: REMOVING,
        floor: Risk::High,
    },
    KindRule {
        group: "apiextensions.k8s.io",
        kind: "CustomResourceDefinition",
        verbs: REMOVING,
        floor: Risk::High,
    },
    KindRule {
        group: RBAC_GROUP,
        kind: "Role",
        verbs: WRITING,
        floor: Risk::Medium,
    },
    KindRule {
        group: RBAC_GROUP,
        kind: "ClusterRole",
        verbs: WRITING,
        floor: Risk::Medium,
    },
    KindRule {
        group: RBAC_GROUP,
        kind: "RoleBinding",
        verbs: WRITING,
        floor: Risk::Medium,
    },
    KindRule {
        group: RBAC_GROUP,
        kind: "ClusterRoleBinding",
        verbs: WRITING,
        floor: Risk::Medium,
    },
];

/// The risk of `verb` on its own, before the kind and flags are considered.
pub const fn verb_floor(verb: MutationVerb) -> Risk {
    match verb {
        MutationVerb::Create
        | MutationVerb::Update
        | MutationVerb::Patch
        | MutationVerb::Apply
        | MutationVerb::Scale
        | MutationVerb::Restart
        | MutationVerb::Cordon
        | MutationVerb::Uncordon => Risk::Low,
        MutationVerb::Delete | MutationVerb::Evict => Risk::Medium,
        MutationVerb::Drain | MutationVerb::DeleteCollection | MutationVerb::Prune => Risk::High,
    }
}

/// Whether `gvk` is a kind shipped with Kubernetes: the core group, or a group with no dot or
/// ending in `.k8s.io`. Anything else is a custom resource whose blast radius is unknown.
fn is_built_in(gvk: &Gvk) -> bool {
    let group = &*gvk.group;
    group.is_empty() || !group.contains('.') || group.ends_with(".k8s.io")
}

/// The risk of one mutation.
///
/// `scope` is whether the kind is namespaced; it raises a write to a cluster-scoped object to
/// at least [`Risk::Medium`] and never changes a delete (a delete is classified by kind, so
/// this agrees with [`crate::command::delete_risk`], which sees no scope). The kind matches on
/// group and kind, not version. An unknown kind is [`Risk::Medium`] for any verb, never
/// [`Risk::Low`].
pub fn classify(verb: MutationVerb, scope: Scope, gvk: &Gvk, flags: MutationFlags) -> Risk {
    let mut risk = verb_floor(verb);
    if !is_built_in(gvk) {
        risk = risk.max(Risk::Medium);
    }
    if let Some(rule) = KIND_RULES.iter().find(|rule| {
        *gvk.group == *rule.group && *gvk.kind == *rule.kind && rule.verbs.contains(&verb)
    }) {
        risk = risk.max(rule.floor);
    }
    let removes = matches!(
        verb,
        MutationVerb::Delete
            | MutationVerb::DeleteCollection
            | MutationVerb::Evict
            | MutationVerb::Drain
            | MutationVerb::Prune
    );
    if scope == Scope::Cluster && !removes {
        risk = risk.max(Risk::Medium);
    }
    if flags.cascading && verb == MutationVerb::Delete {
        risk = risk.max(Risk::High);
    }
    if flags.force {
        risk = risk.max(match verb {
            MutationVerb::Delete | MutationVerb::Evict | MutationVerb::Drain => Risk::High,
            MutationVerb::Apply | MutationVerb::Patch | MutationVerb::Update => Risk::Medium,
            _ => Risk::Low,
        });
    }
    if flags.to_zero && verb == MutationVerb::Scale {
        risk = risk.max(Risk::Medium);
    }
    if flags.all_namespaces {
        risk = risk.max(if removes { Risk::High } else { Risk::Medium });
    }
    risk
}

#[cfg(test)]
mod tests;
