//! Pod columns: the kubectl status rules, ready fractions, restarts, tones.

use oxikube_domain::Resource;
use oxikube_testkit::{fixtures as fx, pod};
use serde_json::json;

use super::{cell, check, now, text};
use crate::columns::{CellSort, Tone};

#[test]
fn status_follows_the_kubectl_printer() {
    let cases: Vec<(Resource, &str)> = vec![
        (fx::pod_running(), "Running"),
        (fx::pod_pending(), "Pending"),
        (fx::pod_container_creating(), "ContainerCreating"),
        (fx::pod_crashloop(), "CrashLoopBackOff"),
        (fx::pod_init(), "Init:1/2"),
        (fx::pod_succeeded(), "Completed"),
        (fx::pod_failed(), "Error"),
        (fx::pod_evicted(), "Evicted"),
        (fx::pod_oom_killed(), "OOMKilled"),
        (fx::pod_err_image_pull(), "ErrImagePull"),
        (fx::pod_image_pull_backoff(), "ImagePullBackOff"),
        (fx::pod_terminating(), "Terminating"),
        (fx::pod_node_lost(), "NodeLost"),
        (fx::pod_sidecar(), "Running"),
        (pod().init(0, 3).build(), "Init:0/3"),
    ];
    for (res, want) in cases {
        assert_eq!(text(&res, "status"), want, "pod {}", res.meta.name);
    }
}

#[test]
fn a_pod_on_a_lost_node_that_is_being_deleted_reads_unknown() {
    let mut json = fx::json("pods/node-lost.json");
    json["metadata"]["deletionTimestamp"] = json!("2026-01-01T01:00:00Z");
    let res = Resource::from_json(json).unwrap();
    assert_eq!(text(&res, "status"), "Unknown");
}

#[test]
fn ready_is_the_container_fraction_and_sorts_by_ratio() {
    check(&fx::pod_running(), &[("ready", "1/1")]);
    check(&fx::pod_pending(), &[("ready", "0/1")]);
    check(&fx::pod_sidecar(), &[("ready", "2/2")]);
    assert_eq!(
        cell(&fx::pod_running(), "ready").sort(),
        CellSort::Float(1.0)
    );
    assert_eq!(
        cell(&fx::pod_pending(), "ready").sort(),
        CellSort::Float(0.0)
    );
}

#[test]
fn restarts_show_the_count_and_when_and_sort_by_the_count() {
    let restarted = fx::pod_running_restarted();
    check(
        &restarted,
        &[("restarts", "3 (26h ago)"), ("last-restart", "26h")],
    );
    assert_eq!(cell(&restarted, "restarts").sort(), CellSort::Int(3));
    check(
        &fx::pod_running(),
        &[("restarts", "0"), ("last-restart", "")],
    );
    assert_eq!(
        cell(&fx::pod_crashloop(), "restarts").sort(),
        CellSort::Int(5)
    );
}

#[test]
fn node_ip_qos_and_owner_columns() {
    let res = fx::pod_running();
    check(
        &res,
        &[
            ("node", "worker-1"),
            ("ip", "10.244.1.10"),
            ("qos", "Burstable"),
            ("controlled-by", "ReplicaSet/web-5d8c7b9f4"),
            ("service-account", "default"),
            ("age", "27h"),
        ],
    );
    // Pending and not scheduled: no node, no IP.
    check(&fx::pod_pending(), &[("node", ""), ("ip", "")]);
}

#[test]
fn ip_prefers_the_first_of_pod_ips() {
    let mut json = fx::json("pods/running.json");
    json["status"]["podIPs"] = json!([{"ip": "fd00::5"}, {"ip": "10.244.1.10"}]);
    json["status"]["podIP"] = json!("10.244.1.10");
    let res = Resource::from_json(json).unwrap();
    assert_eq!(text(&res, "ip"), "fd00::5");
}

#[test]
fn status_tones_follow_meaning() {
    let tone = |res: &Resource| cell(res, "status").tone();
    assert_eq!(tone(&fx::pod_running()), Tone::Ok);
    assert_eq!(tone(&fx::pod_succeeded()), Tone::Ok);
    assert_eq!(tone(&fx::pod_pending()), Tone::Warn);
    assert_eq!(tone(&fx::pod_init()), Tone::Warn);
    assert_eq!(tone(&fx::pod_terminating()), Tone::Warn);
    assert_eq!(tone(&fx::pod_crashloop()), Tone::Error);
    assert_eq!(tone(&fx::pod_failed()), Tone::Error);
    assert_eq!(tone(&fx::pod_image_pull_backoff()), Tone::Error);
    assert_eq!(tone(&fx::pod_oom_killed()), Tone::Error);
    let unknown = pod().running().build();
    let mut json = unknown.into_json();
    json["status"]["containerStatuses"][0]["state"] =
        json!({"terminated": {"reason": "ContainerStatusUnknown", "exitCode": 137}});
    let unknown = Resource::from_json(json).unwrap();
    assert_eq!(text(&unknown, "status"), "ContainerStatusUnknown");
    assert_eq!(tone(&unknown), Tone::Error);
    // Ready: green when complete, amber when short, quiet for a finished pod.
    assert_eq!(cell(&fx::pod_running(), "ready").tone(), Tone::Ok);
    assert_eq!(cell(&fx::pod_pending(), "ready").tone(), Tone::Warn);
    assert_eq!(cell(&fx::pod_succeeded(), "ready").tone(), Tone::Neutral);
}

#[test]
fn metrics_columns_are_pending_hooks_without_a_source() {
    let res = fx::pod_running();
    assert!(cell(&res, "cpu").is_pending());
    assert!(cell(&res, "memory").is_pending());
    assert_eq!(cell(&res, "cpu").display(), "");
}

#[test]
fn now_is_the_reference_clock() {
    // The expected texts above assume this offset from the fixtures' creation time.
    let created: jiff::Timestamp = "2026-01-01T00:00:00Z".parse().unwrap();
    assert_eq!(
        oxikube_domain::Age::between(created, now()).to_string(),
        "27h"
    );
}
