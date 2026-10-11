//! [`Bucket`]: the status group an object falls in on the Workloads overview's bars and in the
//! `-s` table filter (E12-S07).
//!
//! Like [`health_of`](super::health_of), one pure function over the typed view-models, so the bar
//! a user clicks and the rows the table then shows agree. [`bucket_of`] refines the health
//! verdict: it never contradicts it (a [`Health::Healthy`] object is never in a failing bucket).
//!
//! | Kind | Buckets (in the order the bar draws them) |
//! |---|---|
//! | Pod | `Running`, `NotReady` (running, but a container is not ready or it is being deleted), `Pending`, `Succeeded`, `Failed`, `Unknown` |
//! | Deployment, StatefulSet, ReplicaSet, DaemonSet, Node | `Ready`, `NotReady` |
//! | Job | `Running`, `Succeeded`, `Suspended`, `Failed` |
//! | CronJob | `Active`, `Suspended` |

use super::{CronJobSummary, Health, JobStatus, JobSummary, PodPhase, str_of, sub};
use crate::resource::Resource;

/// A status group. The same word means the same thing on every kind that has it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Bucket {
    /// A pod that is running with its containers ready, or a job that has not finished.
    Running,
    /// Short of what is wanted: a running pod with a container not ready, a controller with fewer
    /// ready replicas than desired, a node whose `Ready` condition is not `True`.
    NotReady,
    /// A pod that is accepted but not running yet.
    Pending,
    /// A pod or job that finished successfully.
    Succeeded,
    /// A pod or job that failed.
    Failed,
    /// A pod whose phase is unknown.
    Unknown,
    /// A controller with all its replicas ready, or a node that is ready.
    Ready,
    /// A CronJob that is scheduling.
    Active,
    /// A CronJob or Job that is suspended.
    Suspended,
}

impl Bucket {
    /// Every bucket; the length of a [`Bucket`]-indexed table.
    pub const ALL: [Bucket; 9] = [
        Bucket::Running,
        Bucket::NotReady,
        Bucket::Pending,
        Bucket::Succeeded,
        Bucket::Failed,
        Bucket::Unknown,
        Bucket::Ready,
        Bucket::Active,
        Bucket::Suspended,
    ];

    /// How many buckets there are.
    pub const COUNT: usize = Self::ALL.len();

    /// This bucket's slot in a table of [`Bucket::COUNT`] entries.
    pub const fn index(self) -> usize {
        self as usize
    }

    /// The heading shown beside a bar segment (`Not ready`).
    pub const fn label(self) -> &'static str {
        match self {
            Self::Running => "Running",
            Self::NotReady => "Not ready",
            Self::Pending => "Pending",
            Self::Succeeded => "Succeeded",
            Self::Failed => "Failed",
            Self::Unknown => "Unknown",
            Self::Ready => "Ready",
            Self::Active => "Active",
            Self::Suspended => "Suspended",
        }
    }

    /// The word the `-s` filter takes (`not-ready`).
    pub const fn word(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::NotReady => "not-ready",
            Self::Pending => "pending",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Unknown => "unknown",
            Self::Ready => "ready",
            Self::Active => "active",
            Self::Suspended => "suspended",
        }
    }

    /// The bucket a `-s` word names, ignoring case; `notready` and `not ready` are `not-ready`.
    pub fn parse(word: &str) -> Option<Self> {
        let word = word.trim();
        Self::ALL.into_iter().find(|b| {
            word.eq_ignore_ascii_case(b.word())
                || (*b == Self::NotReady
                    && (word.eq_ignore_ascii_case("notready")
                        || word.eq_ignore_ascii_case("not ready")))
        })
    }

    /// Whether the bucket means something is wrong or short (the bar draws it in a warning or
    /// error tone).
    pub const fn is_problem(self) -> bool {
        matches!(
            self,
            Self::NotReady | Self::Pending | Self::Failed | Self::Unknown
        )
    }
}

/// The buckets of the kind `kind` of API group `group` (empty for core), in drawing order; empty
/// for a kind without a health rule.
pub fn kind_buckets(group: &str, kind: &str) -> &'static [Bucket] {
    use Bucket::{
        Active, Failed, NotReady, Pending, Ready, Running, Succeeded, Suspended, Unknown,
    };
    match (group, kind) {
        ("", "Pod") => &[Running, NotReady, Pending, Succeeded, Failed, Unknown],
        ("", "Node") | ("apps", "Deployment" | "StatefulSet" | "DaemonSet" | "ReplicaSet") => {
            &[Ready, NotReady]
        }
        ("batch", "Job") => &[Running, Succeeded, Suspended, Failed],
        ("batch", "CronJob") => &[Active, Suspended],
        _ => &[],
    }
}

/// The bucket of `res`, given its `health` (from [`health_of`](super::health_of), which the caller
/// already has: a pod's health is the costly part, so it is read once).
///
/// Pure and cheap for the kinds that are tallied per event: pods read one status field and
/// allocate nothing. A job builds its summary, which allocates, and jobs change rarely.
pub fn bucket_of(res: &Resource, health: Health) -> Bucket {
    match (&*res.kind.group, &*res.kind.kind) {
        ("", "Pod") => pod_bucket(res, health),
        ("batch", "Job") => job_bucket(res, health),
        ("batch", "CronJob") => cron_bucket(res, health),
        _ if health.is_healthy() => Bucket::Ready,
        _ => Bucket::NotReady,
    }
}

fn pod_bucket(res: &Resource, health: Health) -> Bucket {
    let status = sub(res.json(), "status");
    let phase = str_of(status, "phase");
    // No phase yet: the pod has not been looked at by the control plane, which is pending.
    match phase.map_or(PodPhase::Pending, |p| PodPhase::parse(Some(p))) {
        PodPhase::Running if health.is_healthy() => Bucket::Running,
        PodPhase::Running => Bucket::NotReady,
        PodPhase::Pending => Bucket::Pending,
        PodPhase::Succeeded => Bucket::Succeeded,
        PodPhase::Failed => Bucket::Failed,
        PodPhase::Unknown => Bucket::Unknown,
    }
}

fn job_bucket(res: &Resource, health: Health) -> Bucket {
    match JobSummary::from_resource(res).map(|j| j.status) {
        Ok(JobStatus::Complete | JobStatus::SuccessCriteriaMet) => Bucket::Succeeded,
        Ok(JobStatus::Failed | JobStatus::FailureTarget) => Bucket::Failed,
        Ok(JobStatus::Suspended) => Bucket::Suspended,
        Ok(JobStatus::Running | JobStatus::Terminating) => Bucket::Running,
        Err(_) if health.is_healthy() => Bucket::Running,
        Err(_) => Bucket::Failed,
    }
}

fn cron_bucket(res: &Resource, health: Health) -> Bucket {
    let suspended = CronJobSummary::from_resource(res).map_or(!health.is_healthy(), |c| c.suspend);
    if suspended {
        Bucket::Suspended
    } else {
        Bucket::Active
    }
}
