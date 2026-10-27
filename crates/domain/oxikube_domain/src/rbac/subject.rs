//! [`Subject`]: who a binding grants to, and [`UserInfo`], who a request is made as.

use std::collections::BTreeSet;
use std::fmt;
use std::sync::Arc;

use crate::json::JsonRef;

/// The group every authenticated request belongs to.
pub const AUTHENTICATED_GROUP: &str = "system:authenticated";
/// The group of requests without credentials.
pub const UNAUTHENTICATED_GROUP: &str = "system:unauthenticated";
/// The group every ServiceAccount belongs to.
pub const SERVICE_ACCOUNTS_GROUP: &str = "system:serviceaccounts";
/// The user name of requests without credentials.
pub const ANONYMOUS_USER: &str = "system:anonymous";
const SERVICE_ACCOUNT_PREFIX: &str = "system:serviceaccount:";

/// Who a RoleBinding or ClusterRoleBinding grants its role to.
///
/// Orders users, then groups, then ServiceAccounts, each by name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Subject {
    /// A user, as the authenticator names it.
    User(Arc<str>),
    /// A group of users.
    Group(Arc<str>),
    /// A ServiceAccount; it authenticates as the user
    /// `system:serviceaccount:<namespace>:<name>`.
    ServiceAccount {
        /// The ServiceAccount's namespace.
        namespace: Arc<str>,
        /// The ServiceAccount's name.
        name: Arc<str>,
    },
}

impl Subject {
    /// A ServiceAccount subject.
    pub fn service_account(namespace: &str, name: &str) -> Self {
        Subject::ServiceAccount {
            namespace: namespace.into(),
            name: name.into(),
        }
    }

    /// The subject a user name stands for: a ServiceAccount for
    /// `system:serviceaccount:<namespace>:<name>`, otherwise a user.
    pub fn from_username(username: &str) -> Self {
        username
            .strip_prefix(SERVICE_ACCOUNT_PREFIX)
            .and_then(|rest| rest.split_once(':'))
            .filter(|(namespace, name)| {
                !namespace.is_empty() && !name.is_empty() && !name.contains(':')
            })
            .map_or_else(
                || Subject::User(username.into()),
                |(namespace, name)| Subject::service_account(namespace, name),
            )
    }

    /// One entry of a binding's `subjects`. A ServiceAccount without a namespace takes
    /// `binding_namespace` (a RoleBinding's own); `None` for an unknown kind, an empty name, or
    /// a ServiceAccount with no namespace either way.
    pub fn from_json(subject: JsonRef<'_>, binding_namespace: Option<&str>) -> Option<Self> {
        let name = subject.get("name").and_then(JsonRef::as_str)?;
        if name.is_empty() {
            return None;
        }
        match subject.get("kind").and_then(JsonRef::as_str)? {
            "User" => Some(Subject::User(name.into())),
            "Group" => Some(Subject::Group(name.into())),
            "ServiceAccount" => {
                let namespace = subject
                    .get("namespace")
                    .and_then(JsonRef::as_str)
                    .filter(|namespace| !namespace.is_empty())
                    .or(binding_namespace)?;
                Some(Subject::service_account(namespace, name))
            }
            _ => None,
        }
    }

    /// `User`, `Group` or `ServiceAccount`.
    pub fn kind(&self) -> &'static str {
        match self {
            Subject::User(_) => "User",
            Subject::Group(_) => "Group",
            Subject::ServiceAccount { .. } => "ServiceAccount",
        }
    }

    /// The user name the subject authenticates as; `None` for a group.
    pub fn username(&self) -> Option<Arc<str>> {
        match self {
            Subject::User(name) => Some(name.clone()),
            Subject::Group(_) => None,
            Subject::ServiceAccount { namespace, name } => {
                Some(format!("{SERVICE_ACCOUNT_PREFIX}{namespace}:{name}").into())
            }
        }
    }

    /// Who a request made as this subject is, with the groups the API server adds on its own:
    ///
    /// - a ServiceAccount is in `system:serviceaccounts` and `system:serviceaccounts:<ns>`;
    /// - every subject but `system:anonymous` (and the `system:unauthenticated` group) is in
    ///   `system:authenticated`, and `system:anonymous` is in `system:unauthenticated`;
    /// - a group stands for a member of it with no user name of their own.
    ///
    /// This is what `kubectl auth can-i --as` sees (`kubectl auth whoami --as`).
    pub fn user_info(&self) -> UserInfo {
        let mut groups = BTreeSet::new();
        match self {
            Subject::Group(group) => {
                groups.insert(group.clone());
            }
            Subject::ServiceAccount { namespace, .. } => {
                groups.insert(Arc::from(SERVICE_ACCOUNTS_GROUP));
                groups.insert(format!("{SERVICE_ACCOUNTS_GROUP}:{namespace}").into());
            }
            Subject::User(_) => {}
        }
        let anonymous = matches!(self, Subject::User(name) if &**name == ANONYMOUS_USER)
            || matches!(self, Subject::Group(group) if &**group == UNAUTHENTICATED_GROUP);
        groups.insert(Arc::from(if anonymous {
            UNAUTHENTICATED_GROUP
        } else {
            AUTHENTICATED_GROUP
        }));
        UserInfo {
            user: self.username(),
            groups,
        }
    }
}

impl fmt::Display for Subject {
    /// `User fx-viewer`, `Group fx-team`, `ServiceAccount demo/builder`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Subject::User(name) | Subject::Group(name) => write!(f, "{} {name}", self.kind()),
            Subject::ServiceAccount { namespace, name } => {
                write!(f, "ServiceAccount {namespace}/{name}")
            }
        }
    }
}

/// Who a request is made as: a user name (none for a bare group) and the groups.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UserInfo {
    /// The user name; `None` when asking for a group's members in general.
    pub user: Option<Arc<str>>,
    /// The groups, sorted.
    pub groups: BTreeSet<Arc<str>>,
}

impl UserInfo {
    /// Whether a binding to `subject` applies to this requester (the API server's
    /// `appliesTo`): a user or ServiceAccount by user name, a group by membership.
    pub fn is(&self, subject: &Subject) -> bool {
        match subject {
            Subject::Group(group) => self.groups.contains(group),
            Subject::User(_) | Subject::ServiceAccount { .. } => {
                self.user.is_some() && self.user == subject.username()
            }
        }
    }

    /// Every binding subject that applies to this requester ([`is`](Self::is) is true for
    /// exactly these): the user (under both spellings for a ServiceAccount: a `User` subject
    /// named `system:serviceaccount:<ns>:<name>` applies too) and each group. Sorted.
    pub fn subjects(&self) -> Vec<Subject> {
        let mut subjects = Vec::with_capacity(self.groups.len() + 2);
        if let Some(user) = &self.user {
            subjects.push(Subject::User(user.clone()));
            let service_account = Subject::from_username(user);
            if matches!(service_account, Subject::ServiceAccount { .. }) {
                subjects.push(service_account);
            }
        }
        subjects.extend(self.groups.iter().cloned().map(Subject::Group));
        subjects.sort();
        subjects
    }
}
