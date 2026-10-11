//! The status buckets behind the Workloads overview's bars and the `-s` filter: each kind's
//! buckets, how an object falls in one, and that a bucket never contradicts the health rule.

use oxikube_domain::Resource;
use oxikube_domain::view::{Bucket, bucket_of, health_of, kind_buckets};
use serde_json::{Value, json};

fn object(api_version: &str, kind: &str, rest: Value) -> Resource {
    let mut json = json!({
        "apiVersion": api_version, "kind": kind,
        "metadata": {"name": "x", "namespace": "d"}
    });
    json.as_object_mut()
        .unwrap()
        .extend(rest.as_object().unwrap().clone());
    Resource::from_json(json).expect("a resource")
}

fn bucket(res: &Resource) -> Bucket {
    bucket_of(res, health_of(res).expect("the kind has a health rule"))
}

fn pod(phase: Option<&str>, ready: bool) -> Resource {
    let status = phase.map_or(json!({}), |p| {
        json!({"phase": p, "containerStatuses": [
            {"name": "a", "ready": ready, "state": {"running": {}}}]})
    });
    object(
        "v1",
        "Pod",
        json!({"spec": {"containers": [{"name": "a"}]}, "status": status}),
    )
}

#[test]
fn pods_fall_in_their_phase_and_a_running_pod_that_is_not_ready_in_not_ready() {
    assert_eq!(bucket(&pod(Some("Running"), true)), Bucket::Running);
    assert_eq!(bucket(&pod(Some("Running"), false)), Bucket::NotReady);
    assert_eq!(bucket(&pod(Some("Pending"), false)), Bucket::Pending);
    assert_eq!(bucket(&pod(Some("Succeeded"), false)), Bucket::Succeeded);
    assert_eq!(bucket(&pod(Some("Failed"), false)), Bucket::Failed);
    assert_eq!(bucket(&pod(Some("Unknown"), false)), Bucket::Unknown);
    assert_eq!(
        bucket(&pod(None, false)),
        Bucket::Pending,
        "not scheduled yet"
    );
}

#[test]
fn controllers_and_nodes_are_ready_or_not_ready() {
    let ds = |desired: u32, ready: u32| {
        object(
            "apps/v1",
            "DaemonSet",
            json!({"status": {"desiredNumberScheduled": desired, "numberReady": ready}}),
        )
    };
    assert_eq!(bucket(&ds(3, 3)), Bucket::Ready);
    assert_eq!(bucket(&ds(3, 1)), Bucket::NotReady);
    let deployment = object(
        "apps/v1",
        "Deployment",
        json!({"spec": {"replicas": 2}, "status": {"readyReplicas": 1}}),
    );
    assert_eq!(bucket(&deployment), Bucket::NotReady);
    let node = object(
        "v1",
        "Node",
        json!({"status": {"conditions": [{"type": "Ready", "status": "True"}]}}),
    );
    assert_eq!(bucket(&node), Bucket::Ready);
}

#[test]
fn jobs_are_running_succeeded_suspended_or_failed_and_cron_jobs_active_or_suspended() {
    let job = |conditions: Value| {
        object(
            "batch/v1",
            "Job",
            json!({"status": {"conditions": conditions}}),
        )
    };
    assert_eq!(bucket(&job(json!([]))), Bucket::Running);
    assert_eq!(
        bucket(&job(json!([{"type": "Complete", "status": "True"}]))),
        Bucket::Succeeded
    );
    assert_eq!(
        bucket(&job(json!([{"type": "Failed", "status": "True"}]))),
        Bucket::Failed
    );
    assert_eq!(
        bucket(&job(json!([{"type": "Suspended", "status": "True"}]))),
        Bucket::Suspended
    );
    let cron = |suspend: bool| {
        object(
            "batch/v1",
            "CronJob",
            json!({"spec": {"schedule": "* * * * *", "suspend": suspend}}),
        )
    };
    assert_eq!(bucket(&cron(false)), Bucket::Active);
    assert_eq!(bucket(&cron(true)), Bucket::Suspended);
}

#[test]
fn a_bucket_is_one_of_its_kinds_buckets_and_agrees_with_health() {
    let samples = [
        ("", "Pod", pod(Some("Running"), true)),
        ("", "Pod", pod(Some("Failed"), false)),
        ("", "Pod", pod(Some("Running"), false)),
        (
            "apps",
            "ReplicaSet",
            object("apps/v1", "ReplicaSet", json!({"spec": {"replicas": 1}})),
        ),
        (
            "batch",
            "CronJob",
            object("batch/v1", "CronJob", json!({"spec": {"suspend": true}})),
        ),
    ];
    for (group, kind, res) in samples {
        let health = health_of(&res).unwrap();
        let b = bucket_of(&res, health);
        assert!(kind_buckets(group, kind).contains(&b), "{kind}: {b:?}");
        if health.is_healthy() {
            assert!(
                !matches!(b, Bucket::NotReady | Bucket::Failed | Bucket::Pending),
                "{kind}: a healthy object is not in {b:?}"
            );
        }
    }
    assert!(kind_buckets("", "ConfigMap").is_empty(), "no rule, no bar");
}

#[test]
fn words_round_trip_and_index_a_table() {
    for (ix, b) in Bucket::ALL.into_iter().enumerate() {
        assert_eq!(Bucket::parse(b.word()), Some(b));
        assert_eq!(Bucket::parse(&b.word().to_uppercase()), Some(b), "case");
        assert_eq!(b.index(), ix, "ALL is in declaration order");
    }
    assert_eq!(Bucket::parse("notready"), Some(Bucket::NotReady));
    assert_eq!(Bucket::parse("not ready"), Some(Bucket::NotReady));
    assert_eq!(Bucket::parse("nope"), None);
    assert_eq!(Bucket::COUNT, 9);
    assert!(Bucket::Failed.is_problem() && !Bucket::Running.is_problem());
}
