//! Eviction retries and the wait for a pod to go: a PodDisruptionBudget that blocks and then
//! allows, timeouts, replaced pods and per-pod failures. The clock is paused, so the backoff
//! and timeouts are asserted exactly and take no real time.

use std::sync::Arc;
use std::time::Duration;

use http::Method;
use oxikube_domain::ErrorKind;
use oxikube_ports::ResourcePort;
use serde_json::json;
use tokio::time::Instant;

use super::drain_support::*;
use super::harness::*;
use crate::algorithms::{DrainOptions, DrainProgress, drain, drain_to_completion};
use crate::fake_api::{FakeApi, status_body};

#[tokio::test(start_paused = true)]
async fn blocked_progress_carries_the_budget_reason_and_the_doubling_delay() {
    let api = server();
    api.reply(NODE, 200, node_json(true));
    api.reply(
        PODS,
        200,
        pod_list(vec![pod_json("a", "n1", Some("ReplicaSet"))]),
    );
    let eviction = eviction_path("a");
    api.reply(&eviction, 429, refused_by_budget());
    api.reply(&eviction, 429, refused_by_budget());
    api.reply(&eviction, 429, refused_by_budget());
    api.reply(&eviction, 201, accepted());
    api.reply(&pod_path("a"), 404, gone());

    let options = DrainOptions {
        retry_initial: Duration::from_secs(1),
        retry_max: Duration::from_secs(2),
        ..DrainOptions::default()
    };
    let started = Instant::now();
    let steps = progress(run(&api, options).await);
    let waited = started.elapsed();

    assert_eq!(
        labels(&steps),
        [
            "Planned",
            "Cordoned(true)",
            "Evicting(a,1)",
            "Blocked(a,1)",
            "Evicting(a,2)",
            "Blocked(a,2)",
            "Evicting(a,3)",
            "Blocked(a,3)",
            "Evicting(a,4)",
            "Evicted(a)",
            "Gone(a)",
            "Finished",
        ]
    );
    let delays: Vec<Duration> = steps
        .iter()
        .filter_map(|s| match s {
            DrainProgress::Blocked {
                retry_in, reason, ..
            } => {
                assert!(
                    reason.contains("disruption budget web needs 2 healthy pods"),
                    "{reason}"
                );
                Some(*retry_in)
            }
            _ => None,
        })
        .collect();
    // 1 s, then doubled to the 2 s cap, which holds.
    assert_eq!(
        delays,
        [
            Duration::from_secs(1),
            Duration::from_secs(2),
            Duration::from_secs(2)
        ]
    );
    assert_eq!(waited, Duration::from_secs(5));
    assert!(finished(&steps).is_complete());
    // No cordon patch: the node was cordoned already.
    assert!(writes(&api).iter().all(|r| r.method == Method::POST));
}

/// Pods `a` (a budget never lets it go) and `b` (evicts at once) on a cordoned node.
fn stuck_budget() -> FakeApi {
    let api = server();
    api.reply(NODE, 200, node_json(true));
    api.reply(
        PODS,
        200,
        pod_list(vec![
            pod_json("a", "n1", Some("ReplicaSet")),
            pod_json("b", "n1", Some("ReplicaSet")),
        ]),
    );
    api.reply(&eviction_path("a"), 429, refused_by_budget());
    api.reply(&eviction_path("b"), 201, accepted());
    api.reply(&pod_path("b"), 404, gone());
    api
}

#[tokio::test(start_paused = true)]
async fn a_budget_that_never_allows_fails_the_pod_at_the_timeout_and_the_drain_goes_on() {
    let options = DrainOptions {
        timeout: Duration::from_secs(5),
        ..DrainOptions::default()
    };
    let started = Instant::now();
    let steps = progress(run(&stuck_budget(), options).await);
    // Waits of 1 s and 2 s fit in the 5 s; a next wait of 4 s would not.
    assert_eq!(started.elapsed(), Duration::from_secs(3));
    let seen = labels(&steps);
    assert!(seen.contains(&"Blocked(a,2)".to_owned()), "{seen:?}");
    assert!(!seen.contains(&"Blocked(a,3)".to_owned()), "{seen:?}");
    assert!(seen.contains(&"PodFailed(a)".to_owned()), "{seen:?}");
    assert!(seen.contains(&"Gone(b)".to_owned()), "{seen:?}");
    let failure = steps
        .iter()
        .find_map(|s| match s {
            DrainProgress::PodFailed { error, .. } => Some(error.clone()),
            _ => None,
        })
        .expect("a failure");
    assert!(
        failure.contains("still blocked at the timeout"),
        "{failure}"
    );
    assert!(failure.contains("disruption budget web"), "{failure}");

    let summary = finished(&steps);
    assert!(!summary.is_complete());
    assert_eq!(
        summary
            .failed
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>(),
        ["a"]
    );
    assert_eq!(
        summary
            .evicted
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>(),
        ["b"]
    );
}

#[tokio::test(start_paused = true)]
async fn drain_to_completion_turns_an_incomplete_drain_into_an_error_naming_the_pods() {
    let options = DrainOptions {
        timeout: Duration::from_secs(5),
        ..DrainOptions::default()
    };
    let port: Arc<dyn ResourcePort> = Arc::new(resources(&stuck_budget()));
    let err = drain_to_completion(drain(port, "n1", options))
        .await
        .expect_err("incomplete");
    assert_eq!(err.kind(), ErrorKind::Conflict);
    assert!(err.to_string().contains("default/a"), "{err}");

    let api = easy(&["a", "b"], node_json(false));
    let port: Arc<dyn ResourcePort> = Arc::new(resources(&api));
    let summary = drain_to_completion(drain(port, "n1", DrainOptions::default()))
        .await
        .expect("complete");
    assert_eq!(summary.evicted.len(), 2);
}

#[tokio::test(start_paused = true)]
async fn an_accepted_eviction_waits_for_the_pod_to_go_and_times_out_if_it_stays() {
    // The pod terminates for two polls, then is gone.
    let api = server();
    api.reply(NODE, 200, node_json(true));
    api.reply(
        PODS,
        200,
        pod_list(vec![pod_json("a", "n1", Some("ReplicaSet"))]),
    );
    api.reply(&eviction_path("a"), 201, accepted());
    api.reply(&pod_path("a"), 200, pod_json("a", "n1", Some("ReplicaSet")));
    api.reply(&pod_path("a"), 200, pod_json("a", "n1", Some("ReplicaSet")));
    api.reply(&pod_path("a"), 404, gone());
    let started = Instant::now();
    let steps = progress(run(&api, DrainOptions::default()).await);
    assert_eq!(started.elapsed(), Duration::from_secs(2));
    assert_eq!(finished(&steps).evicted.len(), 1);

    // A pod that never goes.
    let api = server();
    api.reply(NODE, 200, node_json(true));
    api.reply(
        PODS,
        200,
        pod_list(vec![pod_json("a", "n1", Some("ReplicaSet"))]),
    );
    api.reply(&eviction_path("a"), 201, accepted());
    api.reply(&pod_path("a"), 200, pod_json("a", "n1", Some("ReplicaSet")));
    let options = DrainOptions {
        timeout: Duration::from_secs(3),
        ..DrainOptions::default()
    };
    let steps = progress(run(&api, options).await);
    assert_eq!(
        labels(&steps),
        [
            "Planned",
            "Cordoned(true)",
            "Evicting(a,1)",
            "Evicted(a)",
            "PodFailed(a)",
            "Finished"
        ]
    );
    assert!(!finished(&steps).is_complete());
}

#[tokio::test(start_paused = true)]
async fn a_pod_replaced_under_the_same_name_counts_as_gone() {
    let api = server();
    api.reply(NODE, 200, node_json(true));
    api.reply(
        PODS,
        200,
        pod_list(vec![pod_json("a", "n1", Some("StatefulSet"))]),
    );
    api.reply(&eviction_path("a"), 201, accepted());
    // The StatefulSet recreated `a` at once: same name, another uid.
    let mut replacement = pod_json("a", "n1", Some("StatefulSet"));
    replacement["metadata"]["uid"] = json!("a-new-uid");
    api.reply(&pod_path("a"), 200, replacement);
    let steps = progress(run(&api, DrainOptions::default()).await);
    assert!(finished(&steps).is_complete());
    assert_eq!(finished(&steps).evicted.len(), 1);
}

#[tokio::test(start_paused = true)]
async fn the_uid_precondition_failing_on_a_replaced_pod_counts_as_gone() {
    let api = server();
    api.reply(NODE, 200, node_json(true));
    api.reply(
        PODS,
        200,
        pod_list(vec![pod_json("a", "n1", Some("StatefulSet"))]),
    );
    api.reply(
        &eviction_path("a"),
        409,
        status_body(
            409,
            "Conflict",
            "Precondition failed: UID in precondition: a-uid, UID in object meta: new",
        ),
    );
    let mut replacement = pod_json("a", "n1", Some("StatefulSet"));
    replacement["metadata"]["uid"] = json!("a-new-uid");
    api.reply(&pod_path("a"), 200, replacement);
    let steps = progress(run(&api, DrainOptions::default()).await);
    assert_eq!(
        labels(&steps)[2..],
        ["Evicting(a,1)", "Gone(a)", "Finished"]
    );

    // The same 409 while the pod is still the one planned is a real failure.
    let api = server();
    api.reply(NODE, 200, node_json(true));
    api.reply(
        PODS,
        200,
        pod_list(vec![pod_json("a", "n1", Some("StatefulSet"))]),
    );
    api.reply(
        &eviction_path("a"),
        409,
        status_body(409, "Conflict", "conflict"),
    );
    api.reply(
        &pod_path("a"),
        200,
        pod_json("a", "n1", Some("StatefulSet")),
    );
    let steps = progress(run(&api, DrainOptions::default()).await);
    assert!(labels(&steps).contains(&"PodFailed(a)".to_owned()));
}

#[tokio::test(start_paused = true)]
async fn a_pod_that_is_already_gone_or_forbidden_does_not_stop_the_others() {
    let api = server();
    api.reply(NODE, 200, node_json(true));
    api.reply(
        PODS,
        200,
        pod_list(vec![
            pod_json("a", "n1", Some("ReplicaSet")),
            pod_json("b", "n1", Some("ReplicaSet")),
            pod_json("c", "n1", Some("ReplicaSet")),
        ]),
    );
    api.reply(
        &eviction_path("a"),
        404,
        status_body(404, "NotFound", "pods \"a\" not found"),
    );
    api.reply(
        &eviction_path("b"),
        403,
        status_body(403, "Forbidden", "pods/eviction is forbidden"),
    );
    api.reply(&eviction_path("c"), 201, accepted());
    api.reply(&pod_path("c"), 404, gone());
    let options = DrainOptions {
        concurrency: 1,
        ..DrainOptions::default()
    };
    let steps = progress(run(&api, options).await);
    let seen = labels(&steps);
    assert!(seen.contains(&"Gone(a)".to_owned()), "{seen:?}");
    assert!(seen.contains(&"PodFailed(b)".to_owned()), "{seen:?}");
    assert!(seen.contains(&"Gone(c)".to_owned()), "{seen:?}");
    let DrainProgress::PodFailed { error, .. } = steps
        .iter()
        .find(|s| matches!(s, DrainProgress::PodFailed { .. }))
        .expect("failure")
    else {
        unreachable!()
    };
    assert!(error.contains("forbidden"), "{error}");
    // A forbidden pod is not retried: one request.
    assert_eq!(api.hits(&eviction_path("b")), 1);
}
