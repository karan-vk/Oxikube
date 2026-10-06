//! Job and CronJob cells, from [`JobSummary`] and [`CronJobSummary`].

use jiff::Timestamp;
use oxikube_domain::{Age, CronJobSummary, JobSummary, Resource};

use super::{counted, status_tone};
use crate::columns::{Cell, CellSort};

/// Job `STATUS`: `Complete`, `Failed`, `Running`, ...
pub(crate) fn job_status<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    let Ok(s) = JobSummary::from_resource(res) else {
        return Cell::empty();
    };
    Cell::text(s.status.as_str()).with_tone(status_tone(s.status.as_str()))
}

/// Job `COMPLETIONS`: `succeeded/completions`, sorted by the succeeded count.
pub(crate) fn job_completions<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    let Ok(s) = JobSummary::from_resource(res) else {
        return Cell::empty();
    };
    counted(s.completions_display(), s.succeeded)
}

/// Job `DURATION`: start to completion, or to now while running.
pub(crate) fn job_duration<'a>(res: &'a Resource, now: Timestamp) -> Cell<'a> {
    JobSummary::from_resource(res)
        .ok()
        .and_then(|s| s.duration(now))
        .map_or_else(Cell::empty, Cell::age)
}

/// CronJob `SUSPEND`: `True` or `False`.
pub(crate) fn cron_suspend<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    CronJobSummary::from_resource(res).map_or_else(
        |_| Cell::empty(),
        |s| Cell::text(if s.suspend { "True" } else { "False" }),
    )
}

/// CronJob `ACTIVE`: jobs running now.
pub(crate) fn cron_active<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    CronJobSummary::from_resource(res).map_or_else(
        |_| Cell::empty(),
        |s| Cell::shown(s.active.to_string(), CellSort::Int(i64::from(s.active))),
    )
}

/// CronJob `LAST SCHEDULE`: time since the last run; blank (kubectl `<none>`) if it never ran.
pub(crate) fn cron_last_schedule<'a>(res: &'a Resource, now: Timestamp) -> Cell<'a> {
    CronJobSummary::from_resource(res)
        .ok()
        .and_then(|s| s.last_schedule)
        .map_or_else(Cell::empty, |at| Cell::age(Age::between(at, now)))
}
