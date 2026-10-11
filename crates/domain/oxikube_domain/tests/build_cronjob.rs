//! `job_from_cronjob`: the Job `kubectl create job --from=cronjob/x` builds, pinned as JSON.

use oxikube_domain::build::{INSTANTIATE_ANNOTATION, job_from_cronjob, suspend_patch};
use oxikube_domain::{ErrorKind, Resource};
use serde_json::{Value, json};

fn cronjob(name: &str) -> Value {
    json!({
        "apiVersion": "batch/v1", "kind": "CronJob",
        "metadata": {"name": name, "namespace": "default", "uid": "cj-uid"},
        "spec": {
            "schedule": "0 3 * * *",
            "concurrencyPolicy": "Forbid",
            "jobTemplate": {
                "metadata": {"labels": {"team": "data"}, "annotations": {"note": "x"}},
                "spec": {"backoffLimit": 2, "template": {"spec": {
                    "restartPolicy": "Never",
                    "containers": [{"name": "run", "image": "busybox:1"}],
                }}},
            },
        },
    })
}

fn object(value: Value) -> Resource {
    Resource::from_json(value).expect("a valid object")
}

#[test]
fn the_job_is_the_template_with_a_generated_name_and_an_owner() {
    let job = job_from_cronjob(&object(cronjob("nightly"))).expect("job");
    assert_eq!(
        job,
        json!({
            "apiVersion": "batch/v1",
            "kind": "Job",
            "metadata": {
                "generateName": "nightly-manual-",
                "namespace": "default",
                "labels": {"team": "data"},
                "annotations": {"note": "x", "cronjob.kubernetes.io/instantiate": "manual"},
                "ownerReferences": [{
                    "apiVersion": "batch/v1", "kind": "CronJob", "name": "nightly",
                    "uid": "cj-uid", "controller": true, "blockOwnerDeletion": true,
                }],
            },
            "spec": {"backoffLimit": 2, "template": {"spec": {
                "restartPolicy": "Never",
                "containers": [{"name": "run", "image": "busybox:1"}],
            }}},
        })
    );
    assert_eq!(INSTANTIATE_ANNOTATION, "cronjob.kubernetes.io/instantiate");
}

#[test]
fn the_cronjobs_own_fields_do_not_leak_into_the_job() {
    // kubectl copies the template only: the schedule and the concurrency policy stay behind.
    let job = job_from_cronjob(&object(cronjob("nightly"))).expect("job");
    assert!(job["spec"].get("schedule").is_none());
    assert!(job["spec"].get("concurrencyPolicy").is_none());
}

#[test]
fn a_long_cronjob_name_keeps_the_whole_manual_suffix_within_the_servers_limit() {
    // The longest CronJob name the server allows is 52 characters; the prefix must leave room
    // for the five random characters the server appends to a 63-character name.
    let job = job_from_cronjob(&object(cronjob(&"a".repeat(52)))).expect("job");
    let generate = job["metadata"]["generateName"].as_str().expect("name");
    assert_eq!(generate.len(), 58);
    assert!(generate.ends_with("-manual-"), "{generate}");
    // A name beyond what the server allows is cut, never refused, and never split mid-character.
    let long = job_from_cronjob(&object(cronjob(&"é".repeat(70)))).expect("job");
    let generate = long["metadata"]["generateName"].as_str().expect("name");
    assert!(generate.chars().count() <= 58, "{generate}");
    assert!(generate.ends_with("-manual-"), "{generate}");
}

#[test]
fn a_cronjob_without_a_template_or_uid_is_refused() {
    let mut no_template = cronjob("nightly");
    no_template["spec"]["jobTemplate"] = Value::Null;
    assert_eq!(
        job_from_cronjob(&object(no_template)).unwrap_err().kind(),
        ErrorKind::Validation
    );
    let mut no_uid = cronjob("nightly");
    no_uid["metadata"].as_object_mut().unwrap().remove("uid");
    assert_eq!(
        job_from_cronjob(&object(no_uid)).unwrap_err().kind(),
        ErrorKind::Validation
    );
}

#[test]
fn the_suspend_patch_is_a_merge_patch_on_spec_suspend() {
    assert_eq!(suspend_patch(true), json!({"spec": {"suspend": true}}));
    assert_eq!(suspend_patch(false), json!({"spec": {"suspend": false}}));
}
