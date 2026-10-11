//! `cronjob::Trigger`: the Job made from the template, the dry run first, the names.

use oxikube_domain::ErrorKind;
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::Command;
use serde_json::json;

use super::*;

#[test]
fn a_trigger_creates_a_job_from_the_template_with_a_generated_name_and_an_owner() {
    let t = Workloads::new(false);
    let output = t.run(trigger("nightly"), Initiator::Ui).expect("trigger");

    // The server rehearsed the create, then it was sent.
    assert_eq!(
        t.writes(),
        [
            ("Job".to_owned(), "create", true),
            ("Job".to_owned(), "create", false)
        ]
    );
    let job = t.body(1);
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
    // The rehearsal is the same request.
    assert_eq!(t.body(0), job);

    // The output names the Job, for the toast and for a view to open.
    assert_eq!(
        output.message.as_deref(),
        Some("Triggered CronJob default/nightly: created Job default/nightly-manual-x7k2p")
    );
    let data = output.data.expect("data");
    assert_eq!(data["outcome"], "triggered");
    assert_eq!(data["job"]["name"], "nightly-manual-x7k2p");
    assert_eq!(data["job"]["gvk"]["kind"], "Job");
    assert_eq!(data["open"], data["job"]);
}

#[test]
fn the_job_is_in_the_cluster_afterwards() {
    let t = Workloads::new(false);
    t.run(trigger("nightly"), Initiator::Ui).expect("trigger");
    let jobs = t.objects_of(&Gvk::new("batch", "v1", "Job"));
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].name(), "nightly-manual-x7k2p");
}

#[test]
fn the_cronjobs_concurrency_policy_is_not_consulted() {
    // kubectl creates the Job whatever `concurrencyPolicy` says, and so do we.
    let t = Workloads::new(false);
    assert_eq!(
        cronjob("nightly").get("/spec/concurrencyPolicy").is_some(),
        true
    );
    t.run(trigger("nightly"), Initiator::Ui).expect("trigger");
    assert_eq!(t.writes().len(), 2);
}

#[test]
fn a_long_cronjob_name_leaves_room_for_the_whole_suffix() {
    let t = Workloads::new(false);
    let long = "n".repeat(52);
    t.port().insert(cronjob(&long));
    t.run(trigger(&long), Initiator::Ui).expect("trigger");
    let name = t.body(1)["metadata"]["generateName"]
        .as_str()
        .expect("generateName")
        .to_owned();
    assert!(name.starts_with(&"n".repeat(50)), "{name}");
    assert_eq!(name.len(), 58);
    assert!(name.ends_with("-manual-"), "{name}");
}

#[test]
fn a_dry_run_dispatch_creates_nothing() {
    let t = Workloads::new(false);
    let output = t.dry_run(trigger("nightly")).expect("dry run");
    assert_eq!(t.writes(), [("Job".to_owned(), "create", true)]);
    assert_eq!(
        output.message.as_deref(),
        Some("Dry run: CronJob default/nightly can be triggered")
    );
    let data = output.data.expect("data");
    assert_eq!(data["outcome"], "dry_run");
    assert!(data["open"].is_null(), "nothing to open: {data}");
    assert!(t.objects_of(&Gvk::new("batch", "v1", "Job")).is_empty());
}

#[test]
fn a_rehearsal_the_server_refuses_stops_before_the_real_create() {
    let t = Workloads::new(false);
    t.port()
        .script()
        .create
        .push_err(oxikube_domain::OxiError::forbidden("jobs is forbidden"));
    let error = t.run(trigger("nightly"), Initiator::Ui).unwrap_err();
    assert!(error.to_string().contains("forbidden"), "{error}");
    assert_eq!(t.writes(), [("Job".to_owned(), "create", true)]);
}

#[test]
fn a_missing_cronjob_or_one_without_a_template_is_refused_before_any_write() {
    let t = Workloads::new(false);
    let error = t.run(trigger("ghost"), Initiator::Ui).unwrap_err();
    assert!(
        error.to_string().to_lowercase().contains("not found"),
        "{error}"
    );

    let mut bare = serde_json::to_value(cronjob("bare").to_value()).unwrap();
    bare["spec"]["jobTemplate"] = serde_json::Value::Null;
    t.port()
        .insert(oxikube_domain::Resource::from_json(bare).unwrap());
    let error = t.run(trigger("bare"), Initiator::Ui).unwrap_err();
    assert!(error.to_string().contains("no job template"), "{error}");
    assert!(t.writes().is_empty(), "{:?}", t.writes());
}

#[test]
fn only_a_cronjob_can_be_triggered() {
    let t = Workloads::new(false);
    let deployment = Command::CronJobTrigger {
        target: ResourceRef::namespaced(
            id("a"),
            Gvk::new("apps", "v1", "Deployment"),
            "default",
            "web",
        ),
    };
    let error = t.run(deployment, Initiator::Ui).unwrap_err();
    let _ = ErrorKind::Validation;
    assert!(error.to_string().contains("is not a CronJob"), "{error}");
    assert!(t.writes().is_empty());
}
