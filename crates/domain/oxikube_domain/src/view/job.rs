//! [`JobSummary`] and [`CronJobSummary`].

use std::fmt;
use std::sync::Arc;

use jiff::Timestamp;
use serde_json::Value;

use super::{
    ViewError, arc_of, arr_of, bool_of, check_kind, count_of, opt_count, str_of, sub, ts_of,
};
use crate::age::Age;
use crate::resource::Resource;

/// The `STATUS` column of `kubectl get jobs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JobStatus {
    /// `Complete` condition is `True`.
    Complete,
    /// `Failed` condition is `True`.
    Failed,
    /// `deletionTimestamp` is set.
    Terminating,
    /// `Suspended` condition is `True`.
    Suspended,
    /// `FailureTarget` condition is `True`: the job will fail once its pods are gone.
    FailureTarget,
    /// `SuccessCriteriaMet` condition is `True`: the job will complete once its pods are gone.
    SuccessCriteriaMet,
    /// None of the above.
    Running,
}

impl JobStatus {
    /// The text kubectl prints.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Complete => "Complete",
            Self::Failed => "Failed",
            Self::Terminating => "Terminating",
            Self::Suspended => "Suspended",
            Self::FailureTarget => "FailureTarget",
            Self::SuccessCriteriaMet => "SuccessCriteriaMet",
            Self::Running => "Running",
        }
    }
}

impl fmt::Display for JobStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One row of a job table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobSummary {
    /// `metadata.name`.
    pub name: Arc<str>,
    /// `metadata.namespace`.
    pub namespace: Option<Arc<str>>,
    /// Status, with the printer's precedence: Complete, Failed, Terminating, Suspended,
    /// FailureTarget, SuccessCriteriaMet, Running.
    pub status: JobStatus,
    /// `spec.completions`; `None` means any single success completes the job.
    pub completions: Option<u32>,
    /// `spec.parallelism`.
    pub parallelism: Option<u32>,
    /// `spec.suspend`.
    pub suspend: bool,
    /// `status.active`: pods running now.
    pub active: u32,
    /// `status.succeeded`.
    pub succeeded: u32,
    /// `status.failed`.
    pub failed: u32,
    /// `status.startTime`.
    pub start_time: Option<Timestamp>,
    /// `status.completionTime`; set only when the job succeeded.
    pub completion_time: Option<Timestamp>,
    /// `metadata.creationTimestamp`.
    pub created: Option<Timestamp>,
}

impl JobSummary {
    /// Build the summary of a `batch` Job.
    ///
    /// # Errors
    ///
    /// [`ViewError::WrongKind`] when `res` is not a `batch` Job. Missing or malformed fields
    /// never fail; they fall back to defaults.
    pub fn from_resource(res: &Resource) -> Result<Self, ViewError> {
        check_kind(res, "Job", &[("batch", "Job")])?;
        let spec = sub(&res.json, "spec");
        let status = sub(&res.json, "status");
        let conditions = arr_of(status, "conditions");
        let has = |kind: &str| job_condition(conditions, kind);
        let job_status = if has("Complete") {
            JobStatus::Complete
        } else if has("Failed") {
            JobStatus::Failed
        } else if res.meta.deletion.is_some() {
            JobStatus::Terminating
        } else if has("Suspended") {
            JobStatus::Suspended
        } else if has("FailureTarget") {
            JobStatus::FailureTarget
        } else if has("SuccessCriteriaMet") {
            JobStatus::SuccessCriteriaMet
        } else {
            JobStatus::Running
        };
        Ok(Self {
            name: res.meta.name.clone(),
            namespace: res.meta.namespace.clone(),
            status: job_status,
            completions: opt_count(spec, "completions"),
            parallelism: opt_count(spec, "parallelism"),
            suspend: bool_of(spec, "suspend"),
            active: count_of(status, "active"),
            succeeded: count_of(status, "succeeded"),
            failed: count_of(status, "failed"),
            start_time: ts_of(status, "startTime"),
            completion_time: ts_of(status, "completionTime"),
            created: res.meta.creation,
        })
    }

    /// The `COMPLETIONS` column: `succeeded/completions`, or `succeeded/1` (with ` of N` when
    /// parallelism is above 1) for a job without `spec.completions`.
    pub fn completions_display(&self) -> String {
        match (self.completions, self.parallelism.unwrap_or(0)) {
            (Some(c), _) => format!("{}/{c}", self.succeeded),
            (None, p) if p > 1 => format!("{}/1 of {p}", self.succeeded),
            (None, _) => format!("{}/1", self.succeeded),
        }
    }

    /// The `DURATION` column: start to completion, or start to `now` while unfinished.
    /// `None` before the job starts.
    pub fn duration(&self, now: Timestamp) -> Option<Age> {
        let start = self.start_time?;
        Some(Age::between(start, self.completion_time.unwrap_or(now)))
    }

    /// Age at `now`, if the creation time is known.
    pub fn age(&self, now: Timestamp) -> Option<Age> {
        self.created.map(|c| Age::between(c, now))
    }
}

/// The first condition of `kind` decides, as in the printer.
fn job_condition(conditions: &[Value], kind: &str) -> bool {
    conditions
        .iter()
        .find(|c| str_of(c, "type") == Some(kind))
        .is_some_and(|c| str_of(c, "status") == Some("True"))
}

/// One row of a cron job table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronJobSummary {
    /// `metadata.name`.
    pub name: Arc<str>,
    /// `metadata.namespace`.
    pub namespace: Option<Arc<str>>,
    /// `spec.schedule` in cron syntax; empty when absent.
    pub schedule: Arc<str>,
    /// `spec.timeZone`.
    pub time_zone: Option<Arc<str>>,
    /// `spec.suspend`.
    pub suspend: bool,
    /// Number of entries in `status.active` (jobs running now).
    pub active: u32,
    /// `status.lastScheduleTime`.
    pub last_schedule: Option<Timestamp>,
    /// `status.lastSuccessfulTime`.
    pub last_successful: Option<Timestamp>,
    /// `metadata.creationTimestamp`.
    pub created: Option<Timestamp>,
}

impl CronJobSummary {
    /// Build the summary of a `batch` CronJob.
    ///
    /// # Errors
    ///
    /// [`ViewError::WrongKind`] when `res` is not a `batch` CronJob. Missing or malformed fields
    /// never fail; they fall back to defaults.
    pub fn from_resource(res: &Resource) -> Result<Self, ViewError> {
        check_kind(res, "CronJob", &[("batch", "CronJob")])?;
        let spec = sub(&res.json, "spec");
        let status = sub(&res.json, "status");
        let active = arr_of(status, "active").len();
        Ok(Self {
            name: res.meta.name.clone(),
            namespace: res.meta.namespace.clone(),
            schedule: arc_of(spec, "schedule").unwrap_or_else(|| Arc::from("")),
            time_zone: arc_of(spec, "timeZone"),
            suspend: bool_of(spec, "suspend"),
            active: u32::try_from(active).unwrap_or(u32::MAX),
            last_schedule: ts_of(status, "lastScheduleTime"),
            last_successful: ts_of(status, "lastSuccessfulTime"),
            created: res.meta.creation,
        })
    }

    /// Time since the last schedule at `now`; `None` if it never ran (kubectl prints `<none>`).
    pub fn since_last_schedule(&self, now: Timestamp) -> Option<Age> {
        self.last_schedule.map(|t| Age::between(t, now))
    }

    /// Age at `now`, if the creation time is known.
    pub fn age(&self, now: Timestamp) -> Option<Age> {
        self.created.map(|c| Age::between(c, now))
    }
}
