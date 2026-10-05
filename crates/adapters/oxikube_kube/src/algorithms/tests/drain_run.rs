//! `drain` through `KubeResources` on a scripted API server: the plan, the order of the progress
//! events, concurrency and the failures of the drain as a whole. Time is paused, so waits take
//! no real time.

use std::sync::Arc;

use http::Method;
use oxikube_domain::ErrorKind;
use oxikube_ports::ResourcePort;
use serde_json::json;

use super::drain_support::*;
use super::harness::*;
use crate::algorithms::{BlockReason, DrainOptions, DrainProgress, drain};
use crate::fake_api::{FakeApi, status_body};

#[tokio::test(start_paused = true)]
async fn a_drain_cordons_then_evicts_each_pod_and_skips_daemonset_and_mirror_pods() {
    let api = server();
    api.reply(NODE, 200, node_json(false));
    api.reply(NODE, 200, node_json(true));
    let mirror = {
        let mut pod = pod_json("static", "n1", None);
        pod["metadata"]["annotations"] = json!({"kubernetes.io/config.mirror": "x"});
        pod
    };
    api.reply(
        PODS,
        200,
        pod_list(vec![
            pod_json("a", "n1", Some("ReplicaSet")),
            pod_json("agent", "n1", Some("DaemonSet")),
            mirror,
            pod_json("b", "n1", Some("StatefulSet")),
            // The server honours the field selector; a stray pod on another node is dropped.
            pod_json("elsewhere", "n2", Some("ReplicaSet")),
        ]),
    );
    for name in ["a", "b"] {
        api.reply(&eviction_path(name), 201, accepted());
        api.reply(&pod_path(name), 404, gone());
    }
    let options = DrainOptions {
        ignore_daemonsets: true,
        grace_period_secs: Some(0),
        concurrency: 1,
        ..DrainOptions::default()
    };
    let steps = progress(run(&api, options).await);

    assert_eq!(
        labels(&steps),
        [
            "Planned",
            "Cordoned(false)",
            "Evicting(a,1)",
            "Evicted(a)",
            "Gone(a)",
            "Evicting(b,1)",
            "Evicted(b)",
            "Gone(b)",
            "Finished",
        ]
    );
    let DrainProgress::Planned { evict, skipped } = &steps[0] else {
        panic!("first step");
    };
    assert_eq!(evict.len(), 2);
    assert_eq!(skipped.len(), 2);
    let summary = finished(&steps);
    assert!(summary.is_complete() && !summary.dry_run);
    assert_eq!(summary.node, "n1");
    assert_eq!(summary.evicted.len(), 2);
    assert_eq!(summary.skipped.len(), 2);

    let sent = writes(&api);
    // Cordon first: a merge patch of the node. Then one eviction per evicted pod, none for the
    // DaemonSet or mirror pod.
    assert_eq!(
        sent.iter()
            .map(|r| (r.method.clone(), r.path.clone()))
            .collect::<Vec<_>>(),
        vec![
            (Method::PATCH, NODE.to_owned()),
            (Method::POST, eviction_path("a")),
            (Method::POST, eviction_path("b")),
        ]
    );
    assert_eq!(sent[0].body, Some(json!({"spec": {"unschedulable": true}})));
    assert_eq!(
        sent[0].content_type.as_deref(),
        Some("application/merge-patch+json")
    );
    let options = &sent[1].body.as_ref().expect("body")["deleteOptions"];
    assert_eq!(options["gracePeriodSeconds"], 0);
    assert_eq!(options["preconditions"]["uid"], "a-uid");
    // The pods were listed by node, across namespaces.
    let list = calls(&api)
        .into_iter()
        .find(|r| r.path == PODS)
        .expect("list");
    assert!(
        query(&list).contains(&"fieldSelector=spec.nodeName=n1".to_owned()),
        "{:?}",
        query(&list)
    );
}

#[tokio::test(start_paused = true)]
async fn pods_are_evicted_concurrency_at_a_time() {
    let pods = ["a", "b", "c"];
    let slow = || {
        let api = server();
        api.reply(NODE, 200, node_json(true));
        api.reply(
            PODS,
            200,
            pod_list(
                pods.iter()
                    .map(|n| pod_json(n, "n1", Some("ReplicaSet")))
                    .collect(),
            ),
        );
        for name in pods {
            api.reply(&eviction_path(name), 201, accepted());
            // Terminating for one poll, then gone.
            api.reply(
                &pod_path(name),
                200,
                pod_json(name, "n1", Some("ReplicaSet")),
            );
            api.reply(&pod_path(name), 404, gone());
        }
        api
    };

    let two = DrainOptions {
        concurrency: 2,
        ..DrainOptions::default()
    };
    let seen = labels(&progress(run(&slow(), two).await));
    // a and b are in flight together; c starts only when one of them is gone.
    let first_gone = seen
        .iter()
        .position(|l| l.starts_with("Gone"))
        .expect("a gone");
    assert!(position(&seen, "Evicting(b,1)") < first_gone, "{seen:?}");
    assert!(position(&seen, "Evicting(c,1)") > first_gone, "{seen:?}");

    let one = DrainOptions {
        concurrency: 1,
        ..DrainOptions::default()
    };
    let seen = labels(&progress(run(&slow(), one).await));
    assert!(
        position(&seen, "Evicting(b,1)") > position(&seen, "Gone(a)"),
        "{seen:?}"
    );

    // Zero is read as one, never as "no pod is evicted".
    let zero = DrainOptions {
        concurrency: 0,
        ..DrainOptions::default()
    };
    assert_eq!(
        finished(&progress(run(&slow(), zero).await)).evicted.len(),
        3
    );
}

#[tokio::test(start_paused = true)]
async fn a_pod_that_blocks_the_drain_refuses_it_before_the_node_is_cordoned() {
    let api = server();
    api.reply(NODE, 200, node_json(false));
    api.reply(
        PODS,
        200,
        pod_list(vec![
            pod_json("a", "n1", Some("ReplicaSet")),
            pod_json("lone", "n1", None),
            pod_json("agent", "n1", Some("DaemonSet")),
        ]),
    );
    let steps = run(&api, DrainOptions::default()).await;
    assert_eq!(steps.len(), 1, "a single error ends the stream");
    let err = steps
        .into_iter()
        .next()
        .expect("a step")
        .expect_err("refused");
    assert_eq!(err.kind(), ErrorKind::Validation);
    let text = err.to_string();
    assert!(text.contains("default/lone"), "{text}");
    assert!(text.contains("force"), "{text}");
    assert!(text.contains("1 more"), "{text}");
    assert!(
        writes(&api).is_empty(),
        "nothing is cordoned or evicted: {:?}",
        writes(&api)
    );
    assert!(BlockReason::Unmanaged.explain().contains("force"));
}

/// A node that is schedulable at first; `listings` are what its pod list answers in turn.
fn late_arrival(listings: Vec<Vec<serde_json::Value>>) -> FakeApi {
    let api = server();
    api.reply(NODE, 200, node_json(false));
    api.reply(NODE, 200, node_json(true));
    for pods in listings {
        api.reply(PODS, 200, pod_list(pods));
    }
    for name in ["a", "late"] {
        api.reply(&eviction_path(name), 201, accepted());
        api.reply(&pod_path(name), 404, gone());
    }
    api
}

#[tokio::test(start_paused = true)]
async fn a_pod_scheduled_between_the_list_and_the_cordon_is_still_evicted() {
    let a = pod_json("a", "n1", Some("ReplicaSet"));
    let late = pod_json("late", "n1", Some("ReplicaSet"));
    let api = late_arrival(vec![vec![a.clone()], vec![a, late]]);
    let steps = progress(run(&api, DrainOptions::default()).await);

    let summary = finished(&steps);
    let mut evicted: Vec<String> = summary.evicted.iter().map(ToString::to_string).collect();
    evicted.sort();
    assert_eq!(evicted, ["default/a", "default/late"]);
    assert!(summary.is_complete(), "{summary:?}");
    // The second list came after the cordon.
    let order: Vec<(String, Method)> = calls(&api)
        .into_iter()
        .filter(|r| r.path == PODS || r.method == Method::PATCH)
        .map(|r| (r.path, r.method))
        .collect();
    assert_eq!(
        order,
        [
            (PODS.to_owned(), Method::GET),
            (NODE.to_owned(), Method::PATCH),
            (PODS.to_owned(), Method::GET),
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn a_pod_that_lands_and_blocks_fails_the_drain_with_the_node_cordoned() {
    let a = pod_json("a", "n1", Some("ReplicaSet"));
    let lone = pod_json("lone", "n1", None);
    let api = late_arrival(vec![vec![a.clone()], vec![a, lone]]);
    let steps = run(&api, DrainOptions::default()).await;
    let err = steps
        .into_iter()
        .find_map(Result::err)
        .expect("the drain is refused");
    assert_eq!(err.kind(), ErrorKind::Validation);
    assert!(err.to_string().contains("default/lone"), "{err}");
    let kinds: Vec<Method> = writes(&api).into_iter().map(|r| r.method).collect();
    assert_eq!(kinds, [Method::PATCH], "cordoned, nothing evicted");
}

#[tokio::test(start_paused = true)]
async fn a_node_that_was_already_cordoned_is_listed_once() {
    let api = server();
    api.reply(NODE, 200, node_json(true));
    api.reply(
        PODS,
        200,
        pod_list(vec![pod_json("a", "n1", Some("ReplicaSet"))]),
    );
    api.reply(&eviction_path("a"), 201, accepted());
    api.reply(&pod_path("a"), 404, gone());
    let steps = progress(run(&api, DrainOptions::default()).await);
    assert!(finished(&steps).is_complete());
    let lists = calls(&api).iter().filter(|r| r.path == PODS).count();
    assert_eq!(lists, 1);
}

#[tokio::test(start_paused = true)]
async fn a_dry_run_reports_the_plan_and_changes_nothing() {
    let api = server();
    api.reply(NODE, 200, node_json(false));
    api.reply(
        PODS,
        200,
        pod_list(vec![
            pod_json("a", "n1", Some("ReplicaSet")),
            pod_json("agent", "n1", Some("DaemonSet")),
        ]),
    );
    let options = DrainOptions {
        dry_run: true,
        ignore_daemonsets: true,
        ..DrainOptions::default()
    };
    let steps = progress(run(&api, options).await);
    assert_eq!(labels(&steps), ["Planned", "Finished"]);
    let summary = finished(&steps);
    assert!(summary.dry_run && summary.is_complete());
    assert_eq!(
        summary
            .evicted
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>(),
        ["a"]
    );
    assert_eq!(summary.skipped.len(), 1);
    assert!(writes(&api).is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_missing_node_is_a_single_not_found() {
    let api = server();
    api.reply(
        NODE,
        404,
        status_body(404, "NotFound", "nodes \"n1\" not found"),
    );
    let steps = run(&api, DrainOptions::default()).await;
    assert_eq!(steps.len(), 1);
    let err = steps
        .into_iter()
        .next()
        .expect("step")
        .expect_err("missing");
    assert_eq!(err.kind(), ErrorKind::NotFound);
}

#[tokio::test(start_paused = true)]
async fn a_forbidden_cordon_stops_before_any_eviction() {
    let api = server();
    api.reply(NODE, 200, node_json(false));
    api.reply(
        NODE,
        403,
        status_body(403, "Forbidden", "nodes \"n1\" is forbidden"),
    );
    api.reply(
        PODS,
        200,
        pod_list(vec![pod_json("a", "n1", Some("ReplicaSet"))]),
    );
    let steps = run(&api, DrainOptions::default()).await;
    let seen: Vec<String> = steps
        .iter()
        .map(|s| {
            s.as_ref()
                .map_or_else(|e| format!("Err({:?})", e.kind()), label)
        })
        .collect();
    assert_eq!(seen, ["Planned", "Err(Forbidden)"]);
    assert!(writes(&api).iter().all(|r| r.method == Method::PATCH));
}

#[tokio::test(start_paused = true)]
async fn the_stream_does_nothing_until_it_is_polled() {
    let api = easy(&["a"], node_json(false));
    let port: Arc<dyn ResourcePort> = Arc::new(resources(&api));
    let stream = drain(port, "n1", DrainOptions::default());
    tokio::task::yield_now().await;
    assert!(api.requests().is_empty());
    drop(stream);
    assert!(api.requests().is_empty());
}
