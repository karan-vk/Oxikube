//! [`Objects`]: the RBAC objects the [`SubjectIndex`](super::SubjectIndex) holds, and the one
//! place that edits them from store deltas.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::mem;

use oxikube_domain::rbac::{
    Binding, BindingKind, BindingRef, RbacKind, Role, RoleId, RoleKind, Subject,
};

use crate::store::{ObjectKey, RowChange, RowOp, StoreObject};

/// Roles and bindings by identity, the subject → bindings map, and each kind's rows as the
/// store subscription last had them (its ops are positions).
#[derive(Debug, Default)]
pub(super) struct Objects {
    pub(super) roles: HashMap<RoleId, Role>,
    pub(super) bindings: HashMap<BindingRef, Binding>,
    pub(super) by_subject: BTreeMap<Subject, BTreeSet<BindingRef>>,
    rows: [Vec<ObjectKey>; 4],
}

/// What an apply changed: roles (every role's aggregated rules may move) and/or bindings.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct Changed {
    pub(super) roles: bool,
    pub(super) bindings: bool,
}

impl Changed {
    pub(super) fn any(self) -> bool {
        self.roles || self.bindings
    }
}

impl Objects {
    /// Applies one subscription item of `kind`'s feed. A snapshot replaces that kind only; ops
    /// edit one object each. Ops whose position is out of step with the mirror are skipped
    /// rather than trusted (the next snapshot repairs it).
    pub(super) fn apply(&mut self, kind: RbacKind, rows: &RowChange) -> Changed {
        let mut changed = Changed::default();
        match rows {
            RowChange::Unchanged => {}
            RowChange::Snapshot(all) => {
                for key in mem::take(&mut self.rows[slot(kind)]) {
                    self.remove(kind, &key, &mut changed);
                }
                for object in all {
                    self.rows[slot(kind)].push(ObjectKey::of(object.meta()));
                    self.upsert(kind, object, &mut changed);
                }
            }
            RowChange::Ops(ops) => {
                for op in ops {
                    self.apply_op(kind, op, &mut changed);
                }
            }
        }
        changed
    }

    fn apply_op(&mut self, kind: RbacKind, op: &RowOp, changed: &mut Changed) {
        let at = slot(kind);
        let len = self.rows[at].len();
        match op {
            RowOp::Insert { index, object } if *index <= len => {
                self.rows[at].insert(*index, ObjectKey::of(object.meta()));
                self.upsert(kind, object, changed);
            }
            RowOp::Update { index, object } if *index < len => {
                let key = ObjectKey::of(object.meta());
                let old = mem::replace(&mut self.rows[at][*index], key.clone());
                if old != key {
                    self.remove(kind, &old, changed);
                }
                self.upsert(kind, object, changed);
            }
            RowOp::Remove { index } if *index < len => {
                let key = self.rows[at].remove(*index);
                self.remove(kind, &key, changed);
            }
            RowOp::Insert { .. } | RowOp::Update { .. } | RowOp::Remove { .. } => {}
        }
    }

    /// Reads `object` as `kind` and stores it; an object that does not read (a Table row, a
    /// broken binding) takes out what was stored under its key.
    fn upsert(&mut self, kind: RbacKind, object: &StoreObject, changed: &mut Changed) {
        let key = ObjectKey::of(object.meta());
        let StoreObject::Resource(resource) = object else {
            self.remove(kind, &key, changed);
            return;
        };
        match kind {
            RbacKind::Role | RbacKind::ClusterRole => match Role::from_resource(resource) {
                Ok(role) => {
                    if self.roles.get(&role.id) != Some(&role) {
                        self.roles.insert(role.id.clone(), role);
                        changed.roles = true;
                    }
                }
                Err(_) => self.remove(kind, &key, changed),
            },
            RbacKind::RoleBinding | RbacKind::ClusterRoleBinding => {
                match Binding::from_resource(resource) {
                    Ok(binding) => self.put_binding(binding, changed),
                    Err(_) => self.remove(kind, &key, changed),
                }
            }
        }
    }

    fn put_binding(&mut self, binding: Binding, changed: &mut Changed) {
        if self.bindings.get(&binding.id) == Some(&binding) {
            return;
        }
        if let Some(old) = self.bindings.remove(&binding.id) {
            self.unlink(&old);
        }
        for subject in &binding.subjects {
            self.by_subject
                .entry(subject.clone())
                .or_default()
                .insert(binding.id.clone());
        }
        self.bindings.insert(binding.id.clone(), binding);
        changed.bindings = true;
    }

    fn remove(&mut self, kind: RbacKind, key: &ObjectKey, changed: &mut Changed) {
        match kind {
            RbacKind::Role | RbacKind::ClusterRole => {
                let role_kind = if kind == RbacKind::Role {
                    RoleKind::Role
                } else {
                    RoleKind::ClusterRole
                };
                let id = RoleId {
                    kind: role_kind,
                    namespace: key.namespace.clone(),
                    name: key.name.clone(),
                };
                changed.roles |= self.roles.remove(&id).is_some();
            }
            RbacKind::RoleBinding | RbacKind::ClusterRoleBinding => {
                let binding_kind = if kind == RbacKind::RoleBinding {
                    BindingKind::RoleBinding
                } else {
                    BindingKind::ClusterRoleBinding
                };
                let id = BindingRef {
                    kind: binding_kind,
                    namespace: key.namespace.clone(),
                    name: key.name.clone(),
                };
                if let Some(old) = self.bindings.remove(&id) {
                    self.unlink(&old);
                    changed.bindings = true;
                }
            }
        }
    }

    /// Takes `binding` out of the subject map, dropping subjects left with no binding.
    fn unlink(&mut self, binding: &Binding) {
        for subject in &binding.subjects {
            if let Some(set) = self.by_subject.get_mut(subject) {
                set.remove(&binding.id);
                if set.is_empty() {
                    self.by_subject.remove(subject);
                }
            }
        }
    }

    /// Every ClusterRole, for aggregation.
    pub(super) fn cluster_roles(&self) -> Vec<&Role> {
        self.roles
            .values()
            .filter(|role| role.id.kind == RoleKind::ClusterRole)
            .collect()
    }

    /// How many objects of `kind` the index mirrors.
    pub(super) fn count(&self, kind: RbacKind) -> usize {
        self.rows[slot(kind)].len()
    }
}

fn slot(kind: RbacKind) -> usize {
    match kind {
        RbacKind::Role => 0,
        RbacKind::ClusterRole => 1,
        RbacKind::RoleBinding => 2,
        RbacKind::ClusterRoleBinding => 3,
    }
}
