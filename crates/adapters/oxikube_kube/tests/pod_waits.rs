//! Kind integration for E04-B01: the image-pull-aware pod waits (`common::pods`) and the images
//! the suite runs. The accounting and the status/event readers are pure and run without a
//! cluster; the failure diagnostics run against kind (skipped without `OXIKUBE_TEST_CONTEXT`).

#![cfg(feature = "integration")]

mod common;

use std::time::Duration;

use common::pods::{
    Budget, Failure, bad_image, diagnostics, images_being_pulled, is_pulling, ready, started,
    wait_pod,
};
use k8s_openapi::api::core::v1::{Event, Pod};
use oxikube_kube::NodeShellConfig;
use oxikube_testkit::images;
use oxikube_testkit::integration::TestNamespace;
use serde_json::{Value, json};

const SEC: Duration = Duration::from_secs(1);

fn pod(containers: Value, conditions: Value) -> Pod {
    serde_json::from_value(json!({
        "metadata": {"name": "p"},
        "status": {"phase": "Pending", "containerStatuses": containers, "conditions": conditions},
    }))
    .expect("pod")
}

fn waiting(reason: &str, message: &str) -> Value {
    json!({"name": "c", "image": "i", "imageID": "", "ready": false, "restartCount": 0,
           "state": {"waiting": {"reason": reason, "message": message}}})
}

fn event(reason: &str, message: &str) -> Event {
    serde_json::from_value(json!({
        "metadata": {"name": format!("e-{reason}")}, "reason": reason, "message": message,
        "type": "Normal",
    }))
    .expect("event")
}

#[test]
fn time_spent_pulling_does_not_spend_the_start_budget() {
    let mut budget = Budget::new(30 * SEC, 180 * SEC);
    // Two minutes of pulling: far beyond the 30 s start budget, within the 180 s pull budget.
    for _ in 0..120 {
        assert_eq!(budget.charge(SEC, true), None);
    }
    assert_eq!((budget.active, budget.pulling), (Duration::ZERO, 120 * SEC));
    // Then the pod starts slowly for a different reason: the start budget is intact, and runs out
    // at its own limit.
    for _ in 0..29 {
        assert_eq!(budget.charge(SEC, false), None);
    }
    assert_eq!(budget.charge(SEC, false), Some(Failure::NotReached));
}

#[test]
fn a_pull_that_never_ends_fails_at_the_pull_budget() {
    let mut budget = Budget::new(30 * SEC, 180 * SEC);
    for _ in 0..179 {
        assert_eq!(budget.charge(SEC, true), None);
    }
    assert_eq!(budget.charge(SEC, true), Some(Failure::PullTooSlow));
    assert_eq!(budget.active, Duration::ZERO);
}

#[test]
fn started_and_ready_read_the_pod_status() {
    let running = pod(
        json!([{"name": "c", "image": "i", "imageID": "", "ready": true, "restartCount": 0,
                "state": {"running": {}}}]),
        json!([{"type": "Ready", "status": "True"}]),
    );
    assert!(started(&running) && ready(&running));
    let creating = pod(json!([waiting("ContainerCreating", "")]), json!([]));
    assert!(!started(&creating) && !ready(&creating));
    let no_statuses = pod(json!([]), json!([{"type": "Ready", "status": "False"}]));
    assert!(!started(&no_statuses) && !ready(&no_statuses));
}

#[test]
fn a_pull_in_flight_is_a_pull_a_finished_one_is_not() {
    let creating = pod(json!([waiting("ContainerCreating", "")]), json!([]));
    let pulling = [event("Pulling", r#"Pulling image "busybox:1.37""#)];
    assert_eq!(images_being_pulled(&pulling), ["busybox:1.37"]);
    assert!(is_pulling(&creating, &pulling));

    let pulled = [
        event("Pulling", r#"Pulling image "busybox:1.37""#),
        event(
            "Pulled",
            r#"Successfully pulled image "busybox:1.37" in 1.5s"#,
        ),
    ];
    assert!(images_being_pulled(&pulled).is_empty());
    assert!(!is_pulling(&creating, &pulled));

    // An image already on the node is reported `Pulled` without a pull.
    let present = [event(
        "Pulled",
        r#"Container image "busybox:1.37" already present on machine"#,
    )];
    assert!(!is_pulling(&creating, &present));
    // ContainerCreating with no pull event is the sandbox or volumes, not an image.
    assert!(!is_pulling(&creating, &[]));
}

#[test]
fn a_backing_off_pull_is_pulling_and_a_bad_name_is_permanent() {
    let backoff = pod(
        json!([waiting("ImagePullBackOff", "Back-off pulling")]),
        json!([]),
    );
    assert!(is_pulling(&backoff, &[]));
    assert_eq!(bad_image(&backoff), None);

    let invalid = pod(
        json!([waiting("InvalidImageName", "couldn't parse image")]),
        json!([]),
    );
    assert!(bad_image(&invalid).is_some_and(|b| b.contains("InvalidImageName")));
}

#[test]
fn diagnostics_name_the_container_states_and_the_events() {
    let pod = pod(
        json!([waiting("ErrImagePull", "pull access denied")]),
        json!([]),
    );
    let text = diagnostics(
        Some(&pod),
        &[event("Failed", r#"Failed to pull image "x:1""#)],
    );
    assert!(
        text.contains("container c: ErrImagePull: pull access denied"),
        "{text}"
    );
    assert!(text.contains("Failed: Failed to pull image"), "{text}");
    assert!(diagnostics(None, &[]).contains("pod: not found"));
}

#[test]
fn the_node_shell_default_image_is_a_listed_test_image() {
    assert_eq!(NodeShellConfig::default().image, images::BUSYBOX);
    assert!(images::all().contains(&images::BUSYBOX));
}

#[tokio::test]
async fn an_image_that_cannot_be_pulled_fails_as_a_pull_with_the_cluster_diagnostics() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    let doomed: Pod = serde_json::from_value(json!({
        "metadata": {"name": "doomed"},
        "spec": {"terminationGracePeriodSeconds": 1, "containers": [
            {"name": "c", "image": "registry.invalid/oxikube/none:0"}]},
    }))
    .expect("pod");
    common::logs::create(&client, ns.name(), &doomed).await;

    // The start budget is generous; the pull budget is short. The failure must be the pull's.
    let started_at = std::time::Instant::now();
    let err = wait_pod(
        &client,
        ns.name(),
        "doomed",
        "pod doomed started",
        Budget::new(Duration::from_secs(60), Duration::from_secs(15)),
        started,
    )
    .await
    .expect_err("the image cannot be pulled");
    assert_eq!(err.failure, Failure::PullTooSlow, "{err}");
    assert!(started_at.elapsed() < Duration::from_secs(45), "{err}");
    // Only the moments before the kubelet picked the pod up count as start time.
    assert!(err.active < Duration::from_secs(5), "{err}");
    assert!(err.pulling >= Duration::from_secs(15), "{err}");
    let text = err.to_string();
    assert!(text.contains("registry.invalid/oxikube/none:0"), "{text}");
    assert!(
        text.contains("ErrImagePull") || text.contains("ImagePullBackOff"),
        "{text}"
    );
    assert!(text.contains("events ("), "{text}");
}
