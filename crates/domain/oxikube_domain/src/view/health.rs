//! [`Health`]: whether an object counts as healthy on the overview tiles and sidebar badges.
//!
//! One pure function over the typed view-models, so the tiles, the badges and an agent summary
//! agree. A kind without a rule has no health (`None`): it is counted, never judged.
//!
//! | Kind | Healthy when |
//! |---|---|
//! | Pod | `status.phase` is `Running` or `Succeeded` (`Pending`, `Failed` and `Unknown` are not) |
//! | Deployment, StatefulSet, ReplicaSet, DaemonSet | ready replicas reach the desired count |
//! | Job | it has not failed (`Failed` or `FailureTarget`); running and complete jobs are healthy |
//! | CronJob | it is not suspended (a suspended schedule is not running) |
//! | Node | its `Ready` condition is `True` |
//!
//! A metadata-only object ([`Resource::is_partial`]) has no status, so it has no health either.

use super::{CronJobSummary, JobStatus, JobSummary, NodeSummary, PodPhase};
use super::{WorkloadSummary, str_of, sub};
use crate::resource::Resource;

/// The verdict of [`health_of`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Health {
    /// The object is doing what it should.
    Healthy,
    /// The object is not (pending, failed, not ready, short of replicas).
    Unhealthy,
}

impl Health {
    /// Whether this is [`Health::Healthy`].
    pub const fn is_healthy(self) -> bool {
        matches!(self, Self::Healthy)
    }

    fn of(healthy: bool) -> Self {
        if healthy {
            Self::Healthy
        } else {
            Self::Unhealthy
        }
    }
}

/// Whether [`health_of`] has a rule for the kind `kind` of API group `group` (empty for core).
pub fn has_health_rule(group: &str, kind: &str) -> bool {
    matches!(
        (group, kind),
        ("", "Pod" | "Node")
            | (
                "apps",
                "Deployment" | "StatefulSet" | "DaemonSet" | "ReplicaSet"
            )
            | ("batch", "Job" | "CronJob")
    )
}

/// The health of `res`, or `None` when its kind has no rule or it is a metadata-only object.
///
/// Pure and cheap: it reads the few status fields the rule needs (see the module table).
pub fn health_of(res: &Resource) -> Option<Health> {
    if res.is_partial() || !has_health_rule(&res.kind.group, &res.kind.kind) {
        return None;
    }
    let healthy = match (&*res.kind.group, &*res.kind.kind) {
        ("", "Pod") => {
            // Read the phase directly: building the whole summary would walk every container.
            let phase = PodPhase::parse(str_of(sub(&res.json, "status"), "phase"));
            matches!(phase, PodPhase::Running | PodPhase::Succeeded)
        }
        ("", "Node") => NodeSummary::from_resource(res).ok()?.is_ready(),
        ("batch", "Job") => !matches!(
            JobSummary::from_resource(res).ok()?.status,
            JobStatus::Failed | JobStatus::FailureTarget
        ),
        ("batch", "CronJob") => !CronJobSummary::from_resource(res).ok()?.suspend,
        _ => {
            let w = WorkloadSummary::from_resource(res).ok()?;
            w.ready >= w.desired
        }
    };
    Some(Health::of(healthy))
}
