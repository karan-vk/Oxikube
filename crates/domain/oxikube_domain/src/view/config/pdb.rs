//! [`PdbSummary`]: a PodDisruptionBudget's budget and how many disruptions it allows now.

use std::sync::Arc;

use super::super::{Condition, ViewError, arc_of, arr_of, check_kind, i64_of, sub};
use super::util::{scalar_text, selector_text};
use crate::resource::Resource;

/// A PodDisruptionBudget.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdbSummary {
    /// `spec.minAvailable`: a count (`2`) or a percentage (`50%`).
    pub min_available: Option<Arc<str>>,
    /// `spec.maxUnavailable`: a count or a percentage.
    pub max_unavailable: Option<Arc<str>>,
    /// `spec.selector` as text (`app=web`).
    pub selector: Option<String>,
    /// `spec.unhealthyPodEvictionPolicy`.
    pub unhealthy_policy: Option<Arc<str>>,
    /// `status.currentHealthy`.
    pub current_healthy: i64,
    /// `status.desiredHealthy`.
    pub desired_healthy: i64,
    /// `status.expectedPods`.
    pub expected_pods: i64,
    /// `status.disruptionsAllowed`, when the controller has reported it.
    pub reported_allowed: Option<i64>,
    /// How many pods the budget has seen leave (`status.disruptedPods`).
    pub disrupted_pods: usize,
    /// `status.conditions`.
    pub conditions: Vec<Condition>,
}

impl PdbSummary {
    /// The summary of a `PodDisruptionBudget`.
    ///
    /// # Errors
    ///
    /// [`ViewError::WrongKind`] when `res` is not a PDB.
    pub fn from_resource(res: &Resource) -> Result<Self, ViewError> {
        check_kind(
            res,
            "PodDisruptionBudget",
            &[("policy", "PodDisruptionBudget")],
        )?;
        let (spec, status) = (sub(res.json(), "spec"), sub(res.json(), "status"));
        let text = |key| spec.get(key).and_then(scalar_text).map(Arc::from);
        Ok(Self {
            min_available: text("minAvailable"),
            max_unavailable: text("maxUnavailable"),
            selector: spec.get("selector").and_then(selector_text),
            unhealthy_policy: arc_of(spec, "unhealthyPodEvictionPolicy"),
            current_healthy: i64_of(status, "currentHealthy").unwrap_or(0),
            desired_healthy: i64_of(status, "desiredHealthy").unwrap_or(0),
            expected_pods: i64_of(status, "expectedPods").unwrap_or(0),
            reported_allowed: i64_of(status, "disruptionsAllowed"),
            disrupted_pods: status
                .get("disruptedPods")
                .and_then(|v| v.as_object())
                .map_or(0, |o| o.len()),
            conditions: arr_of(status, "conditions")
                .iter()
                .filter_map(Condition::from_json)
                .collect(),
        })
    }

    /// How many pods may be disrupted now: the controller's `disruptionsAllowed`, else the
    /// healthy pods above the desired count (never negative).
    pub fn allowed_disruptions(&self) -> i64 {
        self.reported_allowed
            .unwrap_or_else(|| (self.current_healthy - self.desired_healthy).max(0))
    }

    /// Whether no pod may be disrupted right now (a drain would wait on this budget).
    pub fn is_blocking(&self) -> bool {
        self.allowed_disruptions() == 0
    }
}
