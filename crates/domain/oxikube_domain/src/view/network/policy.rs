//! [`NetworkPolicySummary`]: a NetworkPolicy with the API's defaults applied.

use std::fmt;

use super::super::{ViewError, arr_of, check_kind, sub};
use super::policy_rule::{PolicyRule, Selection, rules_of};
use crate::resource::Resource;

/// A direction a policy covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyType {
    /// Traffic into the selected pods.
    Ingress,
    /// Traffic out of the selected pods.
    Egress,
}

impl PolicyType {
    /// The API spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            PolicyType::Ingress => "Ingress",
            PolicyType::Egress => "Egress",
        }
    }
}

impl fmt::Display for PolicyType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What a policy does to one direction of the pods it selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyEffect {
    /// The policy does not cover the direction.
    NotCovered,
    /// The direction is covered and no rule allows anything: all of it is denied (a default
    /// deny policy).
    DenyAll,
    /// The direction is covered and a rule allows every peer on every port.
    AllowAll,
    /// The direction is covered and only what the rules allow gets through.
    Restricted,
}

/// A NetworkPolicy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkPolicySummary {
    /// The pods the policy applies to: [`Selection::All`] for the empty selector.
    pub pod_selector: Selection,
    /// The directions covered: `spec.policyTypes`, else `Ingress` plus `Egress` when the policy
    /// has egress rules (the API's defaulting).
    pub policy_types: Vec<PolicyType>,
    /// `spec.ingress`.
    pub ingress: Vec<PolicyRule>,
    /// `spec.egress`.
    pub egress: Vec<PolicyRule>,
}

impl NetworkPolicySummary {
    /// The summary of a `networking.k8s.io` NetworkPolicy.
    ///
    /// # Errors
    ///
    /// [`ViewError::WrongKind`] for any other kind.
    pub fn from_resource(res: &Resource) -> Result<Self, ViewError> {
        check_kind(
            res,
            "NetworkPolicy",
            &[("networking.k8s.io", "NetworkPolicy")],
        )?;
        let spec = sub(res.json(), "spec");
        let ingress = rules_of(spec, "ingress", "from");
        let egress = rules_of(spec, "egress", "to");
        let mut policy_types: Vec<PolicyType> = arr_of(spec, "policyTypes")
            .iter()
            .filter_map(|t| match t.as_str() {
                Some("Ingress") => Some(PolicyType::Ingress),
                Some("Egress") => Some(PolicyType::Egress),
                _ => None,
            })
            .collect();
        if policy_types.is_empty() {
            policy_types.push(PolicyType::Ingress);
            if !egress.is_empty() {
                policy_types.push(PolicyType::Egress);
            }
        }
        let selector = sub(spec, "podSelector");
        Ok(Self {
            pod_selector: Selection::of(selector),
            policy_types,
            ingress,
            egress,
        })
    }

    /// Whether the policy covers `direction`.
    pub fn covers(&self, direction: PolicyType) -> bool {
        self.policy_types.contains(&direction)
    }

    /// What the policy does to `direction` of the selected pods.
    pub fn effect(&self, direction: PolicyType) -> PolicyEffect {
        if !self.covers(direction) {
            return PolicyEffect::NotCovered;
        }
        let rules = match direction {
            PolicyType::Ingress => &self.ingress,
            PolicyType::Egress => &self.egress,
        };
        if rules.is_empty() {
            PolicyEffect::DenyAll
        } else if rules.iter().any(PolicyRule::is_allow_all) {
            PolicyEffect::AllowAll
        } else {
            PolicyEffect::Restricted
        }
    }

    /// The pod selector as one line, `all pods` for the empty selector.
    pub fn pod_selector_text(&self) -> String {
        match &self.pod_selector {
            Selection::All => "all pods".to_owned(),
            Selection::Labels(labels) => labels.clone(),
        }
    }

    /// The policy types as `Ingress, Egress`.
    pub fn policy_types_text(&self) -> String {
        self.policy_types
            .iter()
            .map(|t| t.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
}
