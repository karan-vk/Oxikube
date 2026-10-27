//! [`CanISubject`]: who a can-i question is asked as.

use std::fmt;

use oxikube_domain::access::Impersonate;
use oxikube_domain::{OxiError, OxiResult};

/// Who a question is asked as (`kubectl auth can-i --as`). Held in the panel or the call only;
/// never stored.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum CanISubject {
    /// The current user: a `SelfSubjectAccessReview`.
    #[default]
    Myself,
    /// A user name (`--as jane`).
    User(String),
    /// A group (`--as-group devs`).
    Group(String),
    /// A ServiceAccount (`--as system:serviceaccount:<namespace>:<name>`).
    ServiceAccount {
        /// The ServiceAccount's namespace.
        namespace: String,
        /// Its name.
        name: String,
    },
}

impl CanISubject {
    /// The subject a `kind` word and `name` stand for: `user`, `group` or `serviceaccount`
    /// (`sa`), the latter as `namespace/name` or with `namespace` given.
    ///
    /// # Errors
    ///
    /// A validation error for an unknown kind or an empty name.
    pub fn parse(kind: &str, name: &str, namespace: Option<&str>) -> OxiResult<Self> {
        let name = name.trim();
        if name.is_empty() {
            return Err(OxiError::validation("the subject needs a name"));
        }
        match kind.trim().to_ascii_lowercase().as_str() {
            "user" => Ok(Self::User(name.to_owned())),
            "group" => Ok(Self::Group(name.to_owned())),
            "serviceaccount" | "sa" => {
                let (namespace, name) = match (namespace.map(str::trim), name.split_once('/')) {
                    (Some(ns), _) if !ns.is_empty() => (ns, name),
                    (_, Some((ns, name))) if !ns.is_empty() && !name.is_empty() => (ns, name),
                    _ => {
                        return Err(OxiError::validation(
                            "a ServiceAccount subject needs a namespace (`namespace/name`)",
                        ));
                    }
                };
                Ok(Self::ServiceAccount {
                    namespace: namespace.to_owned(),
                    name: name.to_owned(),
                })
            }
            other => Err(OxiError::validation(format!(
                "unknown subject kind {other:?}: expected user, group or serviceaccount"
            ))),
        }
    }

    /// The identity to ask as; `None` for [`Myself`](Self::Myself).
    pub fn impersonation(&self) -> Option<Impersonate> {
        match self {
            Self::Myself => None,
            Self::User(user) => Some(Impersonate::user(user.clone())),
            Self::Group(group) => Some(Impersonate::group(group.clone())),
            Self::ServiceAccount { namespace, name } => {
                Some(Impersonate::service_account(namespace, name))
            }
        }
    }

    /// The `kubectl` flags that ask as this subject (empty for yourself).
    pub fn kubectl_flags(&self) -> String {
        match self {
            Self::Myself => String::new(),
            Self::User(user) => format!(" --as {user}"),
            Self::Group(group) => format!(" --as-group {group}"),
            Self::ServiceAccount { namespace, name } => {
                format!(" --as system:serviceaccount:{namespace}:{name}")
            }
        }
    }
}

impl fmt::Display for CanISubject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Myself => f.write_str("you"),
            Self::User(user) => write!(f, "user {user}"),
            Self::Group(group) => write!(f, "group {group}"),
            Self::ServiceAccount { namespace, name } => {
                write!(f, "ServiceAccount {namespace}/{name}")
            }
        }
    }
}
