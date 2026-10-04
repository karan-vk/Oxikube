//! `can_i`: a single `SelfSubjectAccessReview`.
//!
//! The precise answer for one action on one object, used where the rules review cannot
//! decide: when it was incomplete, or when a capability is only
//! [restricted](super::CapabilityReport::restricted) to named objects.
//!
//! Like the rules review this is a `POST` that stores nothing; see the [module
//! docs](super) for why `MutationGuard` does not apply.

use k8s_openapi::api::authorization::v1::{
    ResourceAttributes, SelfSubjectAccessReview, SelfSubjectAccessReviewSpec,
    SubjectAccessReviewStatus,
};
use kube::api::PostParams;
use kube::{Api, Client};
use oxikube_domain::OxiResult;
use oxikube_domain::ids::Gvr;

use crate::auth::{CredentialRefresh, classify_with};

/// The action to check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessQuery {
    /// `get`, `list`, `create`, `patch`, `delete`, ...
    pub verb: String,
    /// The resource kind (group, version, plural).
    pub gvr: Gvr,
    /// A subresource such as `exec` or `log`.
    pub subresource: Option<String>,
    /// The namespace; `None` for cluster-scoped checks or "all namespaces".
    pub namespace: Option<String>,
    /// A specific object name; `None` for the kind as a whole.
    pub name: Option<String>,
}

impl AccessQuery {
    /// A check of `verb` on `gvr`, widened with the `with_*` methods.
    pub fn new(verb: impl Into<String>, gvr: Gvr) -> Self {
        Self {
            verb: verb.into(),
            gvr,
            subresource: None,
            namespace: None,
            name: None,
        }
    }

    /// Check a subresource (`exec`, `log`, `portforward`, ...).
    #[must_use]
    pub fn subresource(mut self, subresource: impl Into<String>) -> Self {
        self.subresource = Some(subresource.into());
        self
    }

    /// Check within a namespace.
    #[must_use]
    pub fn namespace(mut self, namespace: impl Into<String>) -> Self {
        self.namespace = Some(namespace.into());
        self
    }

    /// Check one named object.
    #[must_use]
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    fn to_review(&self) -> SelfSubjectAccessReview {
        let non_empty = |s: &str| (!s.is_empty()).then(|| s.to_owned());
        SelfSubjectAccessReview {
            metadata: Default::default(),
            spec: SelfSubjectAccessReviewSpec {
                non_resource_attributes: None,
                resource_attributes: Some(ResourceAttributes {
                    verb: Some(self.verb.clone()),
                    // The core group is the empty string, which the API reads as "".
                    group: Some(self.gvr.group.to_string()),
                    version: non_empty(&self.gvr.version),
                    resource: Some(self.gvr.resource.to_string()),
                    subresource: self.subresource.clone(),
                    namespace: self.namespace.clone(),
                    name: self.name.clone(),
                    ..Default::default()
                }),
            },
            status: None,
        }
    }
}

/// The server's answer to a [`can_i`] check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccessDecision {
    /// The action is allowed.
    Allowed,
    /// The action is not allowed (explicitly denied, or no authorizer allowed it).
    Denied {
        /// The authorizer's reason, when it gave one.
        reason: Option<String>,
    },
    /// The authorizer could not decide (it reported an evaluation error and no allow).
    Unknown {
        /// The evaluation error.
        reason: String,
    },
}

impl AccessDecision {
    /// True only for [`Allowed`](Self::Allowed).
    pub fn is_allowed(&self) -> bool {
        matches!(self, AccessDecision::Allowed)
    }

    fn from_status(status: Option<&SubjectAccessReviewStatus>) -> Self {
        let Some(status) = status else {
            return AccessDecision::Unknown {
                reason: "the server returned no review status".into(),
            };
        };
        if status.allowed {
            return AccessDecision::Allowed;
        }
        match (&status.evaluation_error, status.denied) {
            (Some(err), Some(false) | None) => AccessDecision::Unknown {
                reason: err.clone(),
            },
            _ => AccessDecision::Denied {
                reason: status.reason.clone(),
            },
        }
    }
}

/// Asks the apiserver whether the current user may perform `query`.
///
/// # Errors
///
/// The classified request failure (`Auth`, `Network`, ...). A "no" is not an error: it is
/// [`AccessDecision::Denied`].
pub async fn can_i(
    client: &Client,
    query: &AccessQuery,
    refresh: CredentialRefresh,
) -> OxiResult<AccessDecision> {
    let api: Api<SelfSubjectAccessReview> = Api::all(client.clone());
    let answer = api
        .create(&PostParams::default(), &query.to_review())
        .await
        .map_err(|e| classify_with(&e, refresh))?;
    Ok(AccessDecision::from_status(answer.status.as_ref()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(allowed: bool, denied: Option<bool>, err: Option<&str>) -> SubjectAccessReviewStatus {
        SubjectAccessReviewStatus {
            allowed,
            denied,
            evaluation_error: err.map(str::to_owned),
            reason: Some("because".into()),
        }
    }

    #[test]
    fn review_carries_every_attribute() {
        let q = AccessQuery::new("create", Gvr::new("", "v1", "pods"))
            .subresource("exec")
            .namespace("demo")
            .name("web-0");
        let attrs = q.to_review().spec.resource_attributes.unwrap();
        assert_eq!(attrs.verb.as_deref(), Some("create"));
        assert_eq!(attrs.group.as_deref(), Some(""));
        assert_eq!(attrs.version.as_deref(), Some("v1"));
        assert_eq!(attrs.resource.as_deref(), Some("pods"));
        assert_eq!(attrs.subresource.as_deref(), Some("exec"));
        assert_eq!(attrs.namespace.as_deref(), Some("demo"));
        assert_eq!(attrs.name.as_deref(), Some("web-0"));
    }

    #[test]
    fn cluster_scoped_review_has_no_namespace() {
        let q = AccessQuery::new("list", Gvr::new("", "v1", "nodes"));
        let attrs = q.to_review().spec.resource_attributes.unwrap();
        assert_eq!(attrs.namespace, None);
        assert_eq!(attrs.name, None);
    }

    #[test]
    fn allowed_wins() {
        let d = AccessDecision::from_status(Some(&status(true, None, Some("partial"))));
        assert!(d.is_allowed());
    }

    #[test]
    fn plain_no_is_denied_with_the_reason() {
        let d = AccessDecision::from_status(Some(&status(false, None, None)));
        assert_eq!(
            d,
            AccessDecision::Denied {
                reason: Some("because".into())
            }
        );
        let d = AccessDecision::from_status(Some(&status(false, Some(true), None)));
        assert!(matches!(d, AccessDecision::Denied { .. }));
    }

    #[test]
    fn evaluation_error_without_an_explicit_deny_is_unknown() {
        let d = AccessDecision::from_status(Some(&status(false, None, Some("webhook down"))));
        assert_eq!(
            d,
            AccessDecision::Unknown {
                reason: "webhook down".into()
            }
        );
        let d = AccessDecision::from_status(Some(&status(false, Some(true), Some("webhook down"))));
        assert!(
            matches!(d, AccessDecision::Denied { .. }),
            "an explicit deny is final"
        );
    }

    #[test]
    fn missing_status_is_unknown() {
        assert!(matches!(
            AccessDecision::from_status(None),
            AccessDecision::Unknown { .. }
        ));
    }
}
