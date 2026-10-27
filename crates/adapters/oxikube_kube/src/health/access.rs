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
use oxikube_domain::access::{AccessCheck, AccessDecision};
use oxikube_domain::{OxiError, OxiResult};

use crate::auth::{CredentialRefresh, classify_with};

/// The `SelfSubjectAccessReview` for `check`.
fn to_review(check: &AccessCheck) -> SelfSubjectAccessReview {
    let non_empty = |s: &str| (!s.is_empty()).then(|| s.to_owned());
    SelfSubjectAccessReview {
        metadata: Default::default(),
        spec: SelfSubjectAccessReviewSpec {
            non_resource_attributes: None,
            resource_attributes: Some(ResourceAttributes {
                verb: Some(check.verb.clone()),
                // The core group is the empty string, which the API reads as "".
                group: Some(check.gvr.group.to_string()),
                version: non_empty(&check.gvr.version),
                resource: Some(check.gvr.resource.to_string()),
                subresource: check.subresource.clone(),
                namespace: check.namespace.clone(),
                name: check.name.clone(),
                ..Default::default()
            }),
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
/// The classified request failure (`Auth`, `Network`, ...); `Unsupported` for an impersonated
/// check. A "no" is not an error: it is [`AccessDecision::Denied`].
pub async fn can_i(
    client: &Client,
    check: &AccessCheck,
    refresh: CredentialRefresh,
) -> OxiResult<AccessDecision> {
    if check.impersonate.is_some() {
        return Err(OxiError::unsupported(
            "checking as another identity is not supported yet",
        ));
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
