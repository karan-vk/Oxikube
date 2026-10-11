//! The Job columns E12-S08 adds: `Conditions` and whether a Job finished, plus Completions,
//! Duration and Last schedule as the Jobs and CronJobs tables print them.

use jiff::Timestamp;
use oxikube_domain::{CronJobSummary, JobSummary, Resource, ViewError};
use serde_json::{Value, json};

fn ts(s: &str) -> Timestamp {
    s.parse().unwrap()
}

fn job(spec: Value, status: Value) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "batch/v1", "kind": "Job",
        "metadata": {"name": "import", "namespace": "default",
                     "creationTimestamp": "2026-10-01T00:00:00Z"},
        "spec": spec, "status": status,
    }))
    .unwrap()
}

fn condition(kind: &str, status: &str) -> Value {
    json!({"type": kind, "status": status})
}

#[test]
fn conditions_list_every_true_condition_in_order() {
    let done = job(
        json!({}),
        json!({"conditions": [
            condition("SuccessCriteriaMet", "True"),
            condition("Complete", "True"),
            condition("Failed", "False"),
        ]}),
    );
    assert_eq!(
        JobSummary::conditions_display(&done).unwrap(),
        "SuccessCriteriaMet,Complete"
    );
}

#[test]
fn a_job_without_true_conditions_has_none() {
    let running = job(json!({}), json!({"active": 1}));
    assert_eq!(JobSummary::conditions_display(&running).unwrap(), "");
    let false_only = job(
        json!({}),
        json!({"conditions": [condition("Suspended", "False")]}),
    );
    assert_eq!(JobSummary::conditions_display(&false_only).unwrap(), "");
    // A condition with no type, or a status that is not a string, is skipped, not a panic.
    let odd = job(
        json!({}),
        json!({"conditions": [{"status": "True"}, {"type": "Suspended", "status": true}, 7]}),
    );
    assert_eq!(JobSummary::conditions_display(&odd).unwrap(), "");
}

#[test]
fn conditions_reject_another_kind() {
    let pod = Resource::from_json(json!({
        "apiVersion": "v1", "kind": "Pod", "metadata": {"name": "p"}
    }))
    .unwrap();
    assert!(matches!(
        JobSummary::conditions_display(&pod),
        Err(ViewError::WrongKind { .. })
    ));
}

#[test]
fn only_complete_and_failed_jobs_are_finished() {
    let finished = |status| JobSummary::from_resource(&job(json!({}), status)).unwrap();
    assert!(finished(json!({"conditions": [condition("Complete", "True")]})).is_finished());
    assert!(finished(json!({"conditions": [condition("Failed", "True")]})).is_finished());
    assert!(!finished(json!({"active": 1})).is_finished());
    assert!(!finished(json!({"conditions": [condition("Suspended", "True")]})).is_finished());
    // Its pods are still going away: the controller may yet resume or finish it.
    assert!(!finished(json!({"conditions": [condition("FailureTarget", "True")]})).is_finished());
}

#[test]
fn completions_read_as_succeeded_over_completions() {
    let with = JobSummary::from_resource(&job(json!({"completions": 5}), json!({"succeeded": 2})))
        .unwrap();
    assert_eq!(with.completions_display(), "2/5");
    let any = JobSummary::from_resource(&job(json!({"parallelism": 3}), json!({"succeeded": 1})))
        .unwrap();
    assert_eq!(any.completions_display(), "1/1 of 3");
}

#[test]
fn duration_runs_until_now_then_stops_at_completion() {
    let now = ts("2026-10-01T01:00:00Z");
    let running = JobSummary::from_resource(&job(
        json!({}),
        json!({"startTime": "2026-10-01T00:30:00Z"}),
    ))
    .unwrap();
    assert_eq!(running.duration(now).unwrap().to_kubectl_string(), "30m");
    let done = JobSummary::from_resource(&job(
        json!({}),
        json!({"startTime": "2026-10-01T00:30:00Z", "completionTime": "2026-10-01T00:30:07Z",
               "conditions": [condition("Complete", "True")]}),
    ))
    .unwrap();
    assert_eq!(done.duration(now).unwrap().to_kubectl_string(), "7s");
}

#[test]
fn a_cronjob_reports_its_schedule_zone_and_last_run() {
    let cronjob = Resource::from_json(json!({
        "apiVersion": "batch/v1", "kind": "CronJob",
        "metadata": {"name": "nightly", "namespace": "default"},
        "spec": {"schedule": "*/5 * * * *", "timeZone": "Europe/Paris", "suspend": false},
        "status": {"lastScheduleTime": "2026-10-01T00:55:00Z", "active": [{"name": "nightly-1"}]},
    }))
    .unwrap();
    let summary = CronJobSummary::from_resource(&cronjob).unwrap();
    assert_eq!(&*summary.schedule, "*/5 * * * *");
    assert_eq!(summary.time_zone.as_deref(), Some("Europe/Paris"));
    assert_eq!(summary.active, 1);
    assert_eq!(
        summary
            .since_last_schedule(ts("2026-10-01T01:00:00Z"))
            .unwrap()
            .to_kubectl_string(),
        "5m"
    );
}
