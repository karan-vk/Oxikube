//! [`SubjectSummary`]: the rows of the Users and Groups pages.

use std::collections::BTreeSet;
use std::sync::Arc;

use oxikube_domain::rbac::{RoleKind, Subject};

use crate::access::SubjectIndex;

/// Which page of subjects a list is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubjectList {
    /// Users and ServiceAccounts (a ServiceAccount is a user named
    /// `system:serviceaccount:<ns>:<name>`).
    Users,
    /// Groups.
    Groups,
}

/// One row of the Users or Groups page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubjectSummary {
    /// The subject.
    pub subject: Subject,
    /// How many distinct bindings name it.
    pub bindings: usize,
    /// How many distinct roles those bindings grant.
    pub roles: usize,
}

impl SubjectSummary {
    /// The name the page shows (see [`subject_name`]).
    pub fn name(&self) -> Arc<str> {
        subject_name(&self.subject)
    }
}

/// The name a subject is listed and looked up by: a user or group name, a ServiceAccount's
/// `system:serviceaccount:<ns>:<name>` identity.
///
/// # Panics
///
/// Never in practice: a ServiceAccount always authenticates as a user.
pub fn subject_name(subject: &Subject) -> Arc<str> {
    match subject {
        Subject::User(name) | Subject::Group(name) => name.clone(),
        Subject::ServiceAccount { .. } => subject
            .username()
            .expect("a ServiceAccount authenticates as a user"),
    }
}

/// `User system:serviceaccount:ns:name` is the ServiceAccount `ns/name`.
fn canonical(subject: &Subject) -> Subject {
    match subject {
        Subject::User(name) => Subject::from_username(name),
        other => other.clone(),
    }
}

/// The rows of a Users or Groups page, sorted by name. Reads the bindings only (no role is
/// normalised).
pub fn summaries(index: &SubjectIndex, list: SubjectList) -> Vec<SubjectSummary> {
    let subjects: BTreeSet<Subject> = index
        .subjects()
        .iter()
        .map(canonical)
        .filter(|subject| match list {
            SubjectList::Users => !matches!(subject, Subject::Group(_)),
            SubjectList::Groups => matches!(subject, Subject::Group(_)),
        })
        .collect();
    let mut rows: Vec<SubjectSummary> = subjects
        .into_iter()
        .map(|subject| {
            let bindings = index.bindings_naming(&subject);
            let roles: BTreeSet<(RoleKind, Arc<str>, Option<Arc<str>>)> = bindings
                .iter()
                .map(|binding| {
                    // A Role is the one of the binding's namespace; a ClusterRole is one object.
                    let namespace = match binding.role_ref.kind {
                        RoleKind::Role => binding.id.namespace.clone(),
                        RoleKind::ClusterRole => None,
                    };
                    (
                        binding.role_ref.kind,
                        binding.role_ref.name.clone(),
                        namespace,
                    )
                })
                .collect();
            SubjectSummary {
                subject,
                bindings: bindings.len(),
                roles: roles.len(),
            }
        })
        .collect();
    rows.sort_by_key(SubjectSummary::name);
    rows
}

/// The subject a typed name stands for: a known user or ServiceAccount first, then a known
/// group, otherwise the user (or ServiceAccount, for `system:serviceaccount:<ns>:<name>`) of
/// that name, which no binding names.
pub fn resolve_subject(index: &SubjectIndex, text: &str) -> Subject {
    let text = text.trim();
    let as_user = Subject::from_username(text);
    let known = index.subjects();
    let names = |candidate: &Subject| known.iter().any(|s| canonical(s) == *candidate);
    if names(&as_user) {
        return as_user;
    }
    let as_group = Subject::Group(text.into());
    if names(&as_group) {
        return as_group;
    }
    as_user
}
