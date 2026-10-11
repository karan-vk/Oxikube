//! [`AdmissionPolicySummary`] and [`AdmissionBindingSummary`]: ValidatingAdmissionPolicy and its
//! binding.

use std::sync::Arc;

use super::super::{Condition, ViewError, arc_of, arr_of, check_kind, sub};
use super::util::{TargetRef, selector_text, strings};
use super::webhook::WebhookRule;
use crate::resource::Resource;

const GROUP: &str = "admissionregistration.k8s.io";

/// One CEL validation of a policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyValidation {
    /// The CEL `expression`.
    pub expression: Arc<str>,
    /// The `message` shown on a denial.
    pub message: Option<Arc<str>>,
    /// The `messageExpression` (CEL), when the message is computed.
    pub message_expression: Option<Arc<str>>,
    /// The HTTP `reason` (`Invalid`, `Forbidden`, ...).
    pub reason: Option<Arc<str>>,
}

/// A ValidatingAdmissionPolicy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmissionPolicySummary {
    /// `spec.failurePolicy`; the default is `Fail`.
    pub failure_policy: Arc<str>,
    /// `spec.paramKind`: the kind of the policy's parameters.
    pub param_kind: Option<TargetRef>,
    /// `spec.matchConstraints.resourceRules`.
    pub rules: Vec<WebhookRule>,
    /// `spec.validations`.
    pub validations: Vec<PolicyValidation>,
    /// How many `spec.auditAnnotations`.
    pub audit_annotations: usize,
    /// How many `spec.variables`.
    pub variables: usize,
    /// How many `spec.matchConditions`.
    pub match_conditions: usize,
    /// `status.conditions`.
    pub conditions: Vec<Condition>,
}

impl AdmissionPolicySummary {
    /// The summary of a `ValidatingAdmissionPolicy`.
    ///
    /// # Errors
    ///
    /// [`ViewError::WrongKind`] when `res` is not one.
    pub fn from_resource(res: &Resource) -> Result<Self, ViewError> {
        check_kind(
            res,
            "ValidatingAdmissionPolicy",
            &[(GROUP, "ValidatingAdmissionPolicy")],
        )?;
        let (spec, status) = (sub(res.json(), "spec"), sub(res.json(), "status"));
        Ok(Self {
            failure_policy: arc_of(spec, "failurePolicy").unwrap_or_else(|| Arc::from("Fail")),
            param_kind: spec.get("paramKind").and_then(TargetRef::read),
            rules: arr_of(sub(spec, "matchConstraints"), "resourceRules")
                .iter()
                .map(WebhookRule::read)
                .collect(),
            validations: arr_of(spec, "validations")
                .iter()
                .map(|v| PolicyValidation {
                    expression: arc_of(v, "expression").unwrap_or_else(|| Arc::from("")),
                    message: arc_of(v, "message"),
                    message_expression: arc_of(v, "messageExpression"),
                    reason: arc_of(v, "reason"),
                })
                .collect(),
            audit_annotations: arr_of(spec, "auditAnnotations").len(),
            variables: arr_of(spec, "variables").len(),
            match_conditions: arr_of(spec, "matchConditions").len(),
            conditions: arr_of(status, "conditions")
                .iter()
                .filter_map(Condition::from_json)
                .collect(),
        })
    }
}

/// A binding's `paramRef`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParamRef {
    /// `name`, when the reference is by name.
    pub name: Option<Arc<str>>,
    /// `namespace`, for a namespaced param kind.
    pub namespace: Option<Arc<str>>,
    /// `selector` as text, when the reference is by label selector.
    pub selector: Option<String>,
    /// `parameterNotFoundAction` (`Allow` or `Deny`).
    pub not_found_action: Option<Arc<str>>,
}

/// A ValidatingAdmissionPolicyBinding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmissionBindingSummary {
    /// `spec.policyName`.
    pub policy_name: Arc<str>,
    /// `spec.validationActions` (`Deny`, `Warn`, `Audit`).
    pub validation_actions: Vec<Arc<str>>,
    /// `spec.paramRef`.
    pub param_ref: Option<ParamRef>,
    /// `spec.matchResources.namespaceSelector` as text.
    pub namespace_selector: Option<String>,
    /// `spec.matchResources.objectSelector` as text.
    pub object_selector: Option<String>,
}

impl AdmissionBindingSummary {
    /// The summary of a `ValidatingAdmissionPolicyBinding`.
    ///
    /// # Errors
    ///
    /// [`ViewError::WrongKind`] when `res` is not one.
    pub fn from_resource(res: &Resource) -> Result<Self, ViewError> {
        check_kind(
            res,
            "ValidatingAdmissionPolicyBinding",
            &[(GROUP, "ValidatingAdmissionPolicyBinding")],
        )?;
        let spec = sub(res.json(), "spec");
        let matches = sub(spec, "matchResources");
        Ok(Self {
            policy_name: arc_of(spec, "policyName").unwrap_or_else(|| Arc::from("")),
            validation_actions: strings(spec, "validationActions"),
            param_ref: spec.get("paramRef").map(|p| ParamRef {
                name: arc_of(p, "name"),
                namespace: arc_of(p, "namespace"),
                selector: p.get("selector").and_then(selector_text),
                not_found_action: arc_of(p, "parameterNotFoundAction"),
            }),
            namespace_selector: matches.get("namespaceSelector").and_then(selector_text),
            object_selector: matches.get("objectSelector").and_then(selector_text),
        })
    }

    /// The `Actions` cell: `Deny,Audit`.
    pub fn actions_text(&self) -> String {
        self.validation_actions
            .iter()
            .map(|a| &**a)
            .collect::<Vec<_>>()
            .join(",")
    }
}
