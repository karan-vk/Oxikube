//! [`SubjectIndex`]: subjects → bindings → normalised rules, kept incrementally from the store
//! subscriptions of the four RBAC kinds.
//!
//! # Shape
//!
//! Roles by [`RoleId`], bindings by [`BindingRef`], and `subject -> {binding}` for the subjects
//! the bindings name. A query for a subject expands it to who it authenticates as
//! ([`Subject::user_info`]: a ServiceAccount's user and groups, `system:authenticated`), looks
//! each of those up, and resolves every binding's role to its aggregated, normalised rules.
//!
//! # Cost
//!
//! A delta edits one object: a binding change touches the subject entries of that binding, a
//! role change replaces that role. Nothing is rebuilt per event; a snapshot (the first item of a
//! subscription, a relist) replaces only its own kind. A role's aggregated, normalised rules are
//! computed on first use and cached until any role changes (roles change rarely and aggregation
//! can make one role's change move another's rules). Queries cost O(bindings of the subject)
//! plus the normalisation of their union. The index is plain data with no task of its own: its
//! owner applies deltas and answers queries on a background task, never on the UI thread.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use oxikube_domain::access::Access;
use oxikube_domain::ids::Gvr;
use oxikube_domain::rbac::{
    BindingRef, GrantScope, NormalisedRules, PolicyMatrix, RbacKind, RoleId, Subject,
    aggregated_rules,
};
use parking_lot::Mutex;

use super::grant::{Grant, ScopedRules};
use super::objects::Objects;
use crate::store::{RowChange, StoreDelta};

/// The subject index. Feed it each RBAC kind's subscription deltas with
/// [`apply`](Self::apply); see the [module docs](self).
#[derive(Debug, Default)]
pub struct SubjectIndex {
    objects: Objects,
    /// Aggregated, normalised rules per role; cleared whenever a role changes.
    role_rules: Mutex<HashMap<RoleId, Arc<NormalisedRules>>>,
    version: u64,
}

impl SubjectIndex {
    /// An empty index.
    pub fn new() -> Self {
        Self::default()
    }

    /// Applies one item of `kind`'s subscription. Returns whether any answer may have changed
    /// (then [`version`](Self::version) moved on).
    pub fn apply(&mut self, kind: RbacKind, delta: &StoreDelta) -> bool {
        self.apply_rows(kind, &delta.rows)
    }

    /// Applies one row change of `kind`'s subscription; see [`apply`](Self::apply).
    pub fn apply_rows(&mut self, kind: RbacKind, rows: &RowChange) -> bool {
        let changed = self.objects.apply(kind, rows);
        if changed.roles {
            self.role_rules.get_mut().clear();
        }
        if changed.any() {
            self.version += 1;
        }
        changed.any()
    }

    /// Bumped by every apply that changed something; a view recomputes when it moves.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// How many objects of `kind` the index holds.
    pub fn count(&self, kind: RbacKind) -> usize {
        self.objects.count(kind)
    }

    /// Every subject some binding names, sorted (users, groups, ServiceAccounts). Implicit
    /// groups such as `system:authenticated` appear only when a binding names them.
    pub fn subjects(&self) -> Vec<Subject> {
        self.objects.by_subject.keys().cloned().collect()
    }

    /// The bindings that apply to `subject`, directly or through a group it is in, sorted.
    pub fn bindings_for(&self, subject: &Subject) -> Vec<BindingRef> {
        self.matching(subject).into_keys().collect()
    }

    /// Each binding's grant to `subject`, sorted by binding.
    pub fn grants_for(&self, subject: &Subject) -> Vec<Grant> {
        self.matching(subject)
            .into_iter()
            .filter_map(|(id, via)| {
                let binding = self.objects.bindings.get(&id)?;
                let scope = binding.scope();
                let role = binding.role_id().and_then(|role| self.role_rules(&role));
                let role_found = role.is_some();
                let rules = role.map_or_else(NormalisedRules::default, |rules| match scope {
                    GrantScope::Cluster => (*rules).clone(),
                    GrantScope::Namespace(_) => rules.resource_only(),
                });
                Some(Grant {
                    binding: id,
                    via,
                    role: binding.role_ref.clone(),
                    scope,
                    rules,
                    role_found,
                })
            })
            .collect()
    }

    /// `subject`'s rules grouped by where they hold: the cluster first, then each namespace.
    pub fn rules_for(&self, subject: &Subject) -> Vec<ScopedRules> {
        let mut by_scope: BTreeMap<GrantScope, Vec<NormalisedRules>> = BTreeMap::new();
        for grant in self.grants_for(subject) {
            by_scope.entry(grant.scope).or_default().push(grant.rules);
        }
        by_scope
            .into_iter()
            .map(|(scope, sets)| ScopedRules {
                scope,
                rules: NormalisedRules::union(&sets),
            })
            .collect()
    }

    /// Everything `subject` may do in `namespace` (`None`: cluster-scoped resources and
    /// non-resource URLs, which only cluster-wide grants reach). What a rules review
    /// (`kubectl auth can-i --list -n <namespace>`) lists for the subject.
    pub fn rules_in(&self, subject: &Subject, namespace: Option<&str>) -> NormalisedRules {
        let grants = self.grants_for(subject);
        NormalisedRules::union(
            grants
                .iter()
                .filter(|grant| grant.scope.covers(namespace))
                .map(|grant| &grant.rules),
        )
    }

    /// Whether `subject` may `verb` the resource `gvr` in `namespace` (`None` for a
    /// cluster-scoped resource or across all namespaces), optionally the object `name`.
    /// [`Access::Restricted`] when only named objects are allowed and no name was given.
    pub fn can(
        &self,
        subject: &Subject,
        verb: &str,
        gvr: &Gvr,
        namespace: Option<&str>,
        name: Option<&str>,
    ) -> Access {
        self.rules_in(subject, namespace)
            .level(verb, &gvr.group, &gvr.resource, name)
    }

    /// Whether `subject` may `verb` the non-resource `path` (`/healthz`).
    pub fn can_url(&self, subject: &Subject, verb: &str, path: &str) -> bool {
        self.rules_in(subject, None).allows_url(verb, path)
    }

    /// A role's rules, aggregation included; `None` when the index holds no such role.
    pub fn role_rules(&self, id: &RoleId) -> Option<Arc<NormalisedRules>> {
        if let Some(rules) = self.role_rules.lock().get(id) {
            return Some(rules.clone());
        }
        let role = self.objects.roles.get(id)?;
        let rules = Arc::new(aggregated_rules(role, &self.objects.cluster_roles()));
        self.role_rules.lock().insert(id.clone(), rules.clone());
        Some(rules)
    }

    /// The roles whose rules allow `verb` on `gvr` (on every object or on named ones), sorted.
    pub fn roles_granting(&self, verb: &str, gvr: &Gvr) -> Vec<RoleId> {
        let mut ids: Vec<RoleId> = self.objects.roles.keys().cloned().collect();
        ids.sort();
        ids.retain(|id| {
            self.role_rules(id).is_some_and(|rules| {
                rules
                    .level(verb, &gvr.group, &gvr.resource, None)
                    .is_offered()
            })
        });
        ids
    }

    /// `subject`'s policy matrix where `scope` says (k9s's per-group, per-verb table).
    pub fn matrix(&self, subject: &Subject, scope: &GrantScope) -> PolicyMatrix {
        let namespace = match scope {
            GrantScope::Cluster => None,
            GrantScope::Namespace(namespace) => Some(&**namespace),
        };
        PolicyMatrix::from_rules(&self.rules_in(subject, namespace))
    }

    /// The bindings applying to `subject`, each with the subject entry it matched by.
    fn matching(&self, subject: &Subject) -> BTreeMap<BindingRef, Subject> {
        let mut found = BTreeMap::new();
        for candidate in subject.user_info().subjects() {
            let Some(bindings) = self.objects.by_subject.get(&candidate) else {
                continue;
            };
            for binding in bindings {
                found
                    .entry(binding.clone())
                    .or_insert_with(|| candidate.clone());
            }
        }
        found
    }
}
