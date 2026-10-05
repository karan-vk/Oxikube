//! The drain filters: which pods are evicted, skipped or refused.

use oxikube_domain::Resource;
use serde_json::{Value, json};

use super::harness::*;
use crate::algorithms::{BlockReason, DrainOptions, SkipReason, plan_drain};

fn with(mut pod: Value, patch: Value) -> Resource {
    for (pointer, value) in patch.as_object().expect("patch") {
        let (parent, key) = pointer.rsplit_once('/').expect("pointer");
        pod.pointer_mut(parent).expect("parent")[key] = value.clone();
    }
    object(pod)
}

fn evicted(plan: &crate::algorithms::DrainPlan) -> Vec<&str> {
    plan.evict.iter().map(|p| p.name.as_str()).collect()
}

#[test]
fn managed_pods_are_evicted_in_listing_order() {
    let pods = [
        object(pod_json("b", "n1", Some("ReplicaSet"))),
        object(pod_json("a", "n1", Some("StatefulSet"))),
        object(pod_json("c", "n1", Some("Job"))),
    ];
    let plan = plan_drain(&pods, &DrainOptions::default());
    assert_eq!(evicted(&plan), ["b", "a", "c"]);
    assert!(plan.skipped.is_empty() && plan.blocked.is_empty());
    assert_eq!(plan.evict[0].uid.as_deref(), Some("b-uid"));
    assert_eq!(plan.evict[0].namespace, "default");
}

#[test]
fn daemonset_pods_are_skipped_when_ignored_and_block_otherwise() {
    let pods = [
        object(pod_json("web", "n1", Some("ReplicaSet"))),
        object(pod_json("agent", "n1", Some("DaemonSet"))),
    ];
    let ignoring = DrainOptions {
        ignore_daemonsets: true,
        ..DrainOptions::default()
    };
    let plan = plan_drain(&pods, &ignoring);
    assert_eq!(evicted(&plan), ["web"]);
    assert_eq!(plan.skipped.len(), 1);
    assert_eq!(plan.skipped[0].pod.name, "agent");
    assert_eq!(plan.skipped[0].reason, SkipReason::DaemonSet);

    let plan = plan_drain(&pods, &DrainOptions::default());
    assert_eq!(plan.blocked.len(), 1);
    assert_eq!(plan.blocked[0].pod.name, "agent");
    assert_eq!(plan.blocked[0].reason, BlockReason::DaemonSet);
}

#[test]
fn mirror_pods_are_always_skipped() {
    let mirror = with(
        pod_json("static", "n1", None),
        json!({"/metadata/annotations": {"kubernetes.io/config.mirror": "abc"}}),
    );
    // Even with `force`, and although no controller owns it.
    let force = DrainOptions {
        force: true,
        ..DrainOptions::default()
    };
    for options in [DrainOptions::default(), force] {
        let plan = plan_drain(std::slice::from_ref(&mirror), &options);
        assert!(plan.evict.is_empty() && plan.blocked.is_empty());
        assert_eq!(plan.skipped[0].reason, SkipReason::Mirror);
    }
}

#[test]
fn emptydir_pods_need_the_flag() {
    let pod = with(
        pod_json("cache", "n1", Some("ReplicaSet")),
        json!({"/spec/volumes": [{"name": "scratch", "emptyDir": {}}]}),
    );
    let plan = plan_drain(std::slice::from_ref(&pod), &DrainOptions::default());
    assert_eq!(plan.blocked[0].reason, BlockReason::LocalStorage);
    assert!(plan.evict.is_empty());

    let flagged = DrainOptions {
        delete_emptydir_data: true,
        ..DrainOptions::default()
    };
    let plan = plan_drain(&[pod], &flagged);
    assert_eq!(evicted(&plan), ["cache"]);
    assert!(plan.blocked.is_empty());
}

#[test]
fn other_volume_kinds_are_not_local_storage() {
    let pod = with(
        pod_json("data", "n1", Some("ReplicaSet")),
        json!({"/spec/volumes": [
            {"name": "cfg", "configMap": {"name": "c"}},
            {"name": "disk", "persistentVolumeClaim": {"claimName": "p"}},
        ]}),
    );
    let plan = plan_drain(&[pod], &DrainOptions::default());
    assert_eq!(evicted(&plan), ["data"]);
}

#[test]
fn unmanaged_pods_need_force() {
    let pod = object(pod_json("lone", "n1", None));
    let plan = plan_drain(std::slice::from_ref(&pod), &DrainOptions::default());
    assert_eq!(plan.blocked[0].reason, BlockReason::Unmanaged);

    let force = DrainOptions {
        force: true,
        ..DrainOptions::default()
    };
    assert_eq!(evicted(&plan_drain(&[pod], &force)), ["lone"]);
}

#[test]
fn finished_pods_are_evicted_without_any_flag() {
    for phase in ["Succeeded", "Failed"] {
        let pod = with(
            pod_json("done", "n1", None),
            json!({"/status/phase": phase, "/spec/volumes": [{"name": "s", "emptyDir": {}}]}),
        );
        let plan = plan_drain(&[pod], &DrainOptions::default());
        assert_eq!(evicted(&plan), ["done"], "{phase}");
    }
}

#[test]
fn terminating_pods_are_still_evicted_so_the_drain_waits_for_them() {
    let pod = with(
        pod_json("going", "n1", Some("ReplicaSet")),
        json!({"/metadata/deletionTimestamp": "2026-01-01T00:00:00Z"}),
    );
    assert_eq!(
        evicted(&plan_drain(&[pod], &DrainOptions::default())),
        ["going"]
    );
}

#[test]
fn every_blocker_is_reported_not_only_the_first() {
    let pods = [
        object(pod_json("lone", "n1", None)),
        object(pod_json("agent", "n1", Some("DaemonSet"))),
    ];
    let plan = plan_drain(&pods, &DrainOptions::default());
    let reasons: Vec<_> = plan.blocked.iter().map(|b| b.reason).collect();
    assert_eq!(reasons, [BlockReason::Unmanaged, BlockReason::DaemonSet]);
}
