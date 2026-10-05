//! `trigger_cronjob`: the Job built from the template, and the requests that create it.

use http::Method;
use oxikube_domain::ErrorKind;
use oxikube_ports::WriteOptions;
use serde_json::{Value, json};

use super::harness::*;
use crate::algorithms::{INSTANTIATE_ANNOTATION, job_from_cronjob, trigger_cronjob};
use crate::fake_api::status_body;

fn created_job() -> Value {
    json!({
        "apiVersion": "batch/v1", "kind": "Job",
        "metadata": {"name": "nightly-manual-x7k2p", "namespace": "default", "uid": "j-uid"},
        "spec": {},
    })
}

#[test]
fn the_job_is_the_template_with_a_generated_name_and_an_owner() {
    let job = job_from_cronjob(&object(cronjob_json())).expect("job");
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
fn a_template_without_metadata_still_gets_the_instantiate_annotation() {
    let mut cronjob = cronjob_json();
    cronjob["spec"]["jobTemplate"]
        .as_object_mut()
        .expect("template")
        .remove("metadata");
    let job = job_from_cronjob(&object(cronjob)).expect("job");
    assert_eq!(
        job["metadata"]["annotations"],
        json!({"cronjob.kubernetes.io/instantiate": "manual"})
    );
    assert!(job["metadata"].get("labels").is_none());
}

#[test]
fn a_long_cronjob_name_keeps_the_whole_manual_suffix_within_the_servers_limit() {
    // The longest CronJob name the server allows is 52 characters.
    let mut cronjob = cronjob_json();
    cronjob["metadata"]["name"] = json!("a".repeat(52));
    let job = job_from_cronjob(&object(cronjob)).expect("job");
    let generate = job["metadata"]["generateName"]
        .as_str()
        .expect("generateName");
    assert_eq!(generate.len(), 58);
    assert!(generate.ends_with("-manual-"), "{generate}");
}

#[test]
fn a_cronjob_without_a_template_or_uid_is_refused() {
    let mut no_template = cronjob_json();
    no_template["spec"]
        .as_object_mut()
        .expect("spec")
        .remove("jobTemplate");
    let err = job_from_cronjob(&object(no_template)).expect_err("no template");
    assert_eq!(err.kind(), ErrorKind::Validation);

    let mut no_uid = cronjob_json();
    no_uid["metadata"]
        .as_object_mut()
        .expect("meta")
        .remove("uid");
    let err = job_from_cronjob(&object(no_uid)).expect_err("no uid");
    assert_eq!(err.kind(), ErrorKind::Validation);

    let mut odd = cronjob_json();
    odd["spec"]["jobTemplate"]["metadata"]["labels"] = json!("not a map");
    let err = job_from_cronjob(&object(odd)).expect_err("odd labels");
    assert_eq!(err.kind(), ErrorKind::Validation);
}

#[tokio::test]
async fn trigger_reads_the_cronjob_and_posts_the_job() {
    let api = server();
    api.reply(CRONJOB, 200, cronjob_json());
    api.reply(JOBS, 201, created_job());
    let job = trigger_cronjob(
        &resources(&api),
        "default",
        "nightly",
        &WriteOptions::default(),
    )
    .await
    .expect("trigger");
    assert_eq!(job.name(), "nightly-manual-x7k2p");

    let sent = calls(&api);
    assert_eq!(sent.len(), 2, "{sent:?}");
    assert_eq!(
        (sent[0].method.clone(), sent[0].path.as_str()),
        (Method::GET, CRONJOB)
    );
    assert_eq!(
        (sent[1].method.clone(), sent[1].path.as_str()),
        (Method::POST, JOBS)
    );
    let body = sent[1].body.clone().expect("body");
    assert_eq!(body["metadata"]["generateName"], "nightly-manual-");
    assert_eq!(body["metadata"]["ownerReferences"][0]["uid"], "cj-uid");
    assert_eq!(body["spec"]["backoffLimit"], 2);
    assert!(
        query(&sent[1])
            .iter()
            .all(|pair| !pair.starts_with("dryRun"))
    );
}

#[tokio::test]
async fn a_dry_run_is_sent_to_the_server_as_a_dry_run() {
    let api = server();
    api.reply(CRONJOB, 200, cronjob_json());
    api.reply(JOBS, 201, created_job());
    trigger_cronjob(
        &resources(&api),
        "default",
        "nightly",
        &WriteOptions::dry_run(),
    )
    .await
    .expect("trigger");
    let post = writes(&api).remove(0);
    assert!(
        query(&post).contains(&"dryRun=All".to_owned()),
        "{:?}",
        query(&post)
    );
}

#[tokio::test]
async fn a_missing_cronjob_is_not_found_and_nothing_is_created() {
    let api = server();
    api.reply(
        CRONJOB,
        404,
        status_body(404, "NotFound", "cronjobs.batch \"nightly\" not found"),
    );
    let err = trigger_cronjob(
        &resources(&api),
        "default",
        "nightly",
        &WriteOptions::default(),
    )
    .await
    .expect_err("missing");
    assert_eq!(err.kind(), ErrorKind::NotFound);
    assert!(writes(&api).is_empty());
}

#[tokio::test]
async fn a_forbidden_create_is_forbidden() {
    let api = server();
    api.reply(CRONJOB, 200, cronjob_json());
    api.reply(
        JOBS,
        403,
        status_body(403, "Forbidden", "jobs.batch is forbidden"),
    );
    let err = trigger_cronjob(
        &resources(&api),
        "default",
        "nightly",
        &WriteOptions::default(),
    )
    .await
    .expect_err("forbidden");
    assert_eq!(err.kind(), ErrorKind::Forbidden);
}
