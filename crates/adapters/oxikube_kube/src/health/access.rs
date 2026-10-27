//! `can_i`: a single `SelfSubjectAccessReview`.
//!
//! The precise answer for one action on one object, used where the rules review cannot
//! decide: when it was incomplete, or when a capability is only
//! [restricted](super::CapabilityReport::restricted) to named objects.
//!
//! Like the rules review this is a `POST` that stores nothing; see the [module
//! docs](super) for why `MutationGuard` does not apply.

use k8s_openapi::api::authorization::v1::{
    ResourceAttributes, SelfSubjectAccessReview, SelfSubjectAccessReviewSpec, SubjectAccessReview,
    SubjectAccessReviewSpec, SubjectAccessReviewStatus,
};
use kube::api::PostParams;
use kube::{Api, Client};
use oxikube_domain::access::{AccessCheck, AccessDecision, Impersonate};
use oxikube_domain::{ErrorKind, OxiError, OxiResult};

use crate::auth::{CredentialRefresh, classify_with};

/// The resource attributes of `check`, as both review kinds carry them.
fn attributes(check: &AccessCheck) -> ResourceAttributes {
    let non_empty = |s: &str| (!s.is_empty()).then(|| s.to_owned());
    ResourceAttributes {
        verb: Some(check.verb.clone()),
        // The core group is the empty string, which the API reads as "".
        group: Some(check.gvr.group.to_string()),
        version: non_empty(&check.gvr.version),
        resource: Some(check.gvr.resource.to_string()),
        subresource: check.subresource.clone(),
        namespace: check.namespace.clone(),
        name: check.name.clone(),
        ..Default::default()
    }
}

/// The `SelfSubjectAccessReview` for `check`.
fn to_review(check: &AccessCheck) -> SelfSubjectAccessReview {
    SelfSubjectAccessReview {
        metadata: Default::default(),
        spec: SelfSubjectAccessReviewSpec {
            non_resource_attributes: None,
            resource_attributes: Some(attributes(check)),
        },
        status: None,
    }
}

/// The `SubjectAccessReview` that asks `check` as `identity` (`kubectl auth can-i --as`).
fn to_subject_review(check: &AccessCheck, identity: &Impersonate) -> SubjectAccessReview {
    SubjectAccessReview {
        metadata: Default::default(),
        spec: SubjectAccessReviewSpec {
            user: (!identity.user.is_empty()).then(|| identity.user.clone()),
            groups: (!identity.groups.is_empty()).then(|| identity.groups.clone()),
            resource_attributes: Some(attributes(check)),
            ..Default::default()
        },
        status: None,
    }
}

/// The decision a review's status stands for.
///
/// An allow wins; an evaluation error without an explicit deny is `Unknown`, never `Denied`.
fn decision_from_status(status: Option<&SubjectAccessReviewStatus>) -> AccessDecision {
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

/// Asks the apiserver whether the current user may perform `check`.
///
/// # Errors
///
/// The classified request failure (`Auth`, `Network`, ...). An impersonated check is a
/// `SubjectAccessReview`, which needs `create` on `subjectaccessreviews.authorization.k8s.io`:
/// without it the error is `Forbidden` and says so. A "no" is not an error: it is
/// [`AccessDecision::Denied`].
pub async fn can_i(
    client: &Client,
    check: &AccessCheck,
    refresh: CredentialRefresh,
) -> OxiResult<AccessDecision> {
    if let Some(identity) = &check.impersonate {
        let api: Api<SubjectAccessReview> = Api::all(client.clone());
        let answer = api
            .create(&PostParams::default(), &to_subject_review(check, identity))
            .await
            .map_err(|e| {
                let error = classify_with(&e, refresh);
                if error.kind() == ErrorKind::Forbidden {
                    OxiError::forbidden(
                        "checking as another identity needs permission to create \
                         subjectaccessreviews.authorization.k8s.io, which this user lacks",
                    )
                } else {
                    error
                }
            })?;
        return Ok(decision_from_status(answer.status.as_ref()));
    }
    let api: Api<SelfSubjectAccessReview> = Api::all(client.clone());
    let answer = api
        .create(&PostParams::default(), &to_review(check))
        .await
        .map_err(|e| classify_with(&e, refresh))?;
    Ok(decision_from_status(answer.status.as_ref()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxikube_domain::ids::Gvr;

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
        let q = AccessCheck::new("create", Gvr::new("", "v1", "pods"))
            .subresource("exec")
            .namespace("demo")
            .name("web-0");
        let attrs = to_review(&q).spec.resource_attributes.unwrap();
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
        let q = AccessCheck::new("list", Gvr::new("", "v1", "nodes"));
        let attrs = to_review(&q).spec.resource_attributes.unwrap();
        assert_eq!(attrs.namespace, None);
        assert_eq!(attrs.name, None);
    }

    #[test]
    fn subject_review_carries_the_identity_and_the_attributes() {
        let q = AccessCheck::new("delete", Gvr::new("", "v1", "pods")).namespace("demo");
        let who = Impersonate {
            user: "system:serviceaccount:demo:bot".into(),
            groups: vec!["system:serviceaccounts".into()],
        };
        let review = to_subject_review(&q, &who);
        assert_eq!(
            review.spec.user.as_deref(),
            Some("system:serviceaccount:demo:bot")
        );
        assert_eq!(
            review.spec.groups.as_deref(),
            Some(&["system:serviceaccounts".to_owned()][..])
        );
        let attrs = review.spec.resource_attributes.expect("attributes");
        assert_eq!(attrs.verb.as_deref(), Some("delete"));
        assert_eq!(attrs.namespace.as_deref(), Some("demo"));
    }

    #[test]
    fn subject_review_leaves_empty_user_and_groups_unset() {
        let q = AccessCheck::new("get", Gvr::new("", "v1", "pods"));
        let who = Impersonate {
            user: String::new(),
            groups: vec!["devs".into()],
        };
        let review = to_subject_review(&q, &who);
        assert_eq!(review.spec.user, None);
        assert_eq!(
            review.spec.groups.as_deref(),
            Some(&["devs".to_owned()][..])
        );
    }

    #[test]
    fn allowed_wins() {
        let d = decision_from_status(Some(&status(true, None, Some("partial"))));
        assert!(d.is_allowed());
    }

    #[test]
    fn plain_no_is_denied_with_the_reason() {
        let d = decision_from_status(Some(&status(false, None, None)));
        assert_eq!(
            d,
            AccessDecision::Denied {
                reason: Some("because".into())
            }
        );
        let d = decision_from_status(Some(&status(false, Some(true), None)));
        assert!(matches!(d, AccessDecision::Denied { .. }));
    }

    #[test]
    fn evaluation_error_without_an_explicit_deny_is_unknown() {
        let d = decision_from_status(Some(&status(false, None, Some("webhook down"))));
        assert_eq!(
            d,
            AccessDecision::Unknown {
                reason: "webhook down".into()
            }
        );
        let d = decision_from_status(Some(&status(false, Some(true), Some("webhook down"))));
        assert!(
            matches!(d, AccessDecision::Denied { .. }),
            "an explicit deny is final"
        );
    }

    #[test]
    fn missing_status_is_unknown() {
        assert!(matches!(
            decision_from_status(None),
            AccessDecision::Unknown { .. }
        ));
    }
}
