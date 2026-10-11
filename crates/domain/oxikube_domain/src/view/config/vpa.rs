//! [`VpaSummary`]: a VerticalPodAutoscaler (an optional CRD, `autoscaling.k8s.io`).

use std::sync::Arc;

use super::super::{Condition, ViewError, arc_of, arr_of, check_kind, opt_count, sub};
use super::util::{TargetRef, quantities, strings};
use crate::resource::Resource;

/// The recommendation for one container: bounds and target per resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VpaRecommendation {
    /// The container.
    pub container: Arc<str>,
    /// `lowerBound` as `(resource, quantity)` pairs.
    pub lower: Vec<(Arc<str>, Arc<str>)>,
    /// `target`.
    pub target: Vec<(Arc<str>, Arc<str>)>,
    /// `upperBound`.
    pub upper: Vec<(Arc<str>, Arc<str>)>,
}

/// What `spec.resourcePolicy` says about one container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VpaContainerPolicy {
    /// The container (`*` for all).
    pub container: Arc<str>,
    /// `mode`: `Auto` or `Off`.
    pub mode: Option<Arc<str>>,
    /// `minAllowed`.
    pub min_allowed: Vec<(Arc<str>, Arc<str>)>,
    /// `maxAllowed`.
    pub max_allowed: Vec<(Arc<str>, Arc<str>)>,
    /// `controlledResources`.
    pub controlled: Vec<Arc<str>>,
}

/// A VerticalPodAutoscaler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VpaSummary {
    /// `spec.targetRef`.
    pub target: Option<TargetRef>,
    /// `spec.updatePolicy.updateMode`; the API default is `Auto`.
    pub mode: Arc<str>,
    /// `spec.updatePolicy.minReplicas`.
    pub min_replicas: Option<u32>,
    /// `status.conditions`.
    pub conditions: Vec<Condition>,
    /// `status.recommendation.containerRecommendations`.
    pub recommendations: Vec<VpaRecommendation>,
    /// `spec.resourcePolicy.containerPolicies`.
    pub policies: Vec<VpaContainerPolicy>,
}

impl VpaSummary {
    /// The summary of a `VerticalPodAutoscaler`.
    ///
    /// # Errors
    ///
    /// [`ViewError::WrongKind`] when `res` is not a VPA.
    pub fn from_resource(res: &Resource) -> Result<Self, ViewError> {
        check_kind(
            res,
            "VerticalPodAutoscaler",
            &[("autoscaling.k8s.io", "VerticalPodAutoscaler")],
        )?;
        let (spec, status) = (sub(res.json(), "spec"), sub(res.json(), "status"));
        let update = sub(spec, "updatePolicy");
        let pairs = |v, key| quantities(v, key).into_iter().collect::<Vec<_>>();
        Ok(Self {
            target: spec.get("targetRef").and_then(TargetRef::read),
            mode: arc_of(update, "updateMode").unwrap_or_else(|| Arc::from("Auto")),
            min_replicas: opt_count(update, "minReplicas"),
            conditions: arr_of(status, "conditions")
                .iter()
                .filter_map(Condition::from_json)
                .collect(),
            recommendations: arr_of(sub(status, "recommendation"), "containerRecommendations")
                .iter()
                .map(|r| VpaRecommendation {
                    container: arc_of(r, "containerName").unwrap_or_else(|| Arc::from("")),
                    lower: pairs(r, "lowerBound"),
                    target: pairs(r, "target"),
                    upper: pairs(r, "upperBound"),
                })
                .collect(),
            policies: arr_of(sub(spec, "resourcePolicy"), "containerPolicies")
                .iter()
                .map(|p| VpaContainerPolicy {
                    container: arc_of(p, "containerName").unwrap_or_else(|| Arc::from("*")),
                    mode: arc_of(p, "mode"),
                    min_allowed: pairs(p, "minAllowed"),
                    max_allowed: pairs(p, "maxAllowed"),
                    controlled: strings(p, "controlledResources"),
                })
                .collect(),
        })
    }

    /// The `Conditions` cell: the types of the conditions that hold, comma-separated.
    pub fn conditions_text(&self) -> String {
        self.conditions
            .iter()
            .filter(|c| c.is_true())
            .map(|c| &*c.kind)
            .collect::<Vec<_>>()
            .join(",")
    }
}
