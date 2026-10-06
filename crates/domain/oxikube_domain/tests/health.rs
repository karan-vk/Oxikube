//! The health rule behind the overview tiles and sidebar badges: Pod phases, workload ready vs
//! desired, Job success and failure, CronJob suspension, Node readiness, and the kinds that have
//! no rule.

use oxikube_domain::Resource;
use oxikube_domain::view::{Health, has_health_rule, health_of};
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

fn pod(phase: Option<&str>) -> Resource {
    let status = phase.map_or(json!({}), |p| json!({"phase": p}));
    object("v1", "Pod", json!({"status": status}))
}

fn workload(kind: &str, desired: u32, ready: u32) -> Resource {
    object(
        "apps/v1",
        kind,
        json!({"spec": {"replicas": desired}, "status": {"readyReplicas": ready}}),
    )
}

fn job(conditions: Value) -> Resource {
    object(
        "batch/v1",
        "Job",
        json!({"status": {"conditions": conditions}}),
    )
}

fn cron(suspend: bool) -> Resource {
    object(
        "batch/v1",
        "CronJob",
        json!({"spec": {"schedule": "0 3 * * *", "suspend": suspend}}),
    )
}

fn node(ready: &str) -> Resource {
    object(
        "v1",
        "Node",
        json!({"status": {"conditions": [{"type": "Ready", "status": ready}]}}),
    )
}

#[test]
fn pod_phases() {
    assert_eq!(health_of(&pod(Some("Running"))), Some(Health::Healthy));
    assert_eq!(health_of(&pod(Some("Succeeded"))), Some(Health::Healthy));
    assert_eq!(health_of(&pod(Some("Pending"))), Some(Health::Unhealthy));
    assert_eq!(health_of(&pod(Some("Failed"))), Some(Health::Unhealthy));
    assert_eq!(health_of(&pod(Some("Unknown"))), Some(Health::Unhealthy));
    // No phase at all (a pod the API server has not scheduled yet) is not healthy.
    assert_eq!(health_of(&pod(None)), Some(Health::Unhealthy));
}

#[test]
fn workloads_are_healthy_when_ready_reaches_desired() {
    for kind in ["Deployment", "StatefulSet", "ReplicaSet"] {
        assert_eq!(health_of(&workload(kind, 3, 3)), Some(Health::Healthy));
        assert_eq!(health_of(&workload(kind, 3, 2)), Some(Health::Unhealthy));
        // Scaled to zero wants nothing, so nothing is missing.
        assert_eq!(health_of(&workload(kind, 0, 0)), Some(Health::Healthy));
    }
    let ds = |desired: u32, ready: u32| {
        object(
            "apps/v1",
            "DaemonSet",
            json!({"status": {"desiredNumberScheduled": desired, "numberReady": ready}}),
        )
    };
    assert_eq!(health_of(&ds(2, 2)), Some(Health::Healthy));
    assert_eq!(health_of(&ds(2, 1)), Some(Health::Unhealthy));
}

#[test]
fn jobs_are_unhealthy_only_when_they_failed() {
    assert_eq!(health_of(&job(json!([]))), Some(Health::Healthy), "running");
    let complete = json!([{"type": "Complete", "status": "True"}]);
    assert_eq!(health_of(&job(complete)), Some(Health::Healthy));
    let failed = json!([{"type": "Failed", "status": "True"}]);
    assert_eq!(health_of(&job(failed)), Some(Health::Unhealthy));
    let failing = json!([{"type": "FailureTarget", "status": "True"}]);
    assert_eq!(health_of(&job(failing)), Some(Health::Unhealthy));
}

#[test]
fn cron_jobs_are_healthy_unless_suspended() {
    assert_eq!(health_of(&cron(false)), Some(Health::Healthy));
    assert_eq!(health_of(&cron(true)), Some(Health::Unhealthy));
}

#[test]
fn nodes_follow_their_ready_condition() {
    assert_eq!(health_of(&node("True")), Some(Health::Healthy));
    assert_eq!(health_of(&node("False")), Some(Health::Unhealthy));
}

#[test]
fn kinds_without_a_rule_and_partial_objects_have_no_health() {
    let config_map = object("v1", "ConfigMap", json!({}));
    assert_eq!(health_of(&config_map), None);
    assert!(!has_health_rule("", "ConfigMap"));
    assert!(has_health_rule("apps", "Deployment"));
    assert!(!has_health_rule("example.com", "Deployment"));
    let partial = pod(Some("Running")).into_partial();
    assert_eq!(health_of(&partial), None, "metadata-only has no status");
    assert!(Health::Healthy.is_healthy() && !Health::Unhealthy.is_healthy());
}
