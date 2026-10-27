//! [`Escalation`]: the bindings a confirmation should single out, kept here (the classification
//! table) so no view decides what is dangerous.

use super::role::{RoleKind, RoleRef};
use super::subject::{
    ANONYMOUS_USER, AUTHENTICATED_GROUP, SERVICE_ACCOUNTS_GROUP, Subject, UNAUTHENTICATED_GROUP,
};

/// The ClusterRole that allows everything.
pub const CLUSTER_ADMIN: &str = "cluster-admin";

/// Why a grant deserves a louder warning than an ordinary binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Escalation {
    /// The binding names `cluster-admin`: every verb on every resource.
    ClusterAdmin,
    /// A subject stands for everybody (`system:authenticated`, `system:unauthenticated`,
    /// `system:anonymous`) or every ServiceAccount (`system:serviceaccounts`).
    BroadSubject,
}

impl Escalation {
    /// What to tell the user.
    pub fn message(self) -> &'static str {
        match self {
            Escalation::ClusterAdmin => {
                "cluster-admin allows every action on every resource in its scope."
            }
            Escalation::BroadSubject => {
                "A subject here stands for a very large group (every authenticated user, anonymous \
                 requests or every ServiceAccount), so the role is granted to all of them."
            }
        }
    }

    /// The escalations of a binding of `role` to `subjects`, the worst first.
    pub fn of(role: &RoleRef, subjects: &[Subject]) -> Vec<Escalation> {
        let mut found = Vec::new();
        if role.kind == RoleKind::ClusterRole && &*role.name == CLUSTER_ADMIN {
            found.push(Escalation::ClusterAdmin);
        }
        if subjects.iter().any(is_broad) {
            found.push(Escalation::BroadSubject);
        }
        found
    }
}

fn is_broad(subject: &Subject) -> bool {
    match subject {
        Subject::Group(group) => matches!(
            &**group,
            AUTHENTICATED_GROUP | UNAUTHENTICATED_GROUP | SERVICE_ACCOUNTS_GROUP
        ),
        Subject::User(name) => &**name == ANONYMOUS_USER,
        Subject::ServiceAccount { .. } => false,
    }
}
