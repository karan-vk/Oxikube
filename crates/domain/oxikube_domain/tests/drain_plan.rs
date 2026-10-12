//! The drain filters (`kubectl drain`): which pods are evicted, skipped or refused.

use oxikube_domain::Resource;
use oxikube_domain::drain::{BlockReason, DrainOptions, DrainPlan, SkipReason, plan_drain};
use serde_json::{Value, json};

fn object(value: Value) -> Resource {
    Resource::from_json(value).expect("a valid object")
}

/// A Running pod on `node` in `default`, controlled by an `owner_kind` when given.
fn pod_json(name: &str, node: &str, owner_kind: Option<&str>) -> Value {
    let mut pod = json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": {"name": name, "namespace": "default", "uid": format!("{name}-uid")},
        "spec": {"nodeName": node, "containers": [{"name": "c", "image": "pause"}]},
        "status": {"phase": "Running"},
    });
    if let Some(kind) = owner_kind {
        pod["metadata"]["ownerReferences"] = json!([{
            "apiVersion": "apps/v1", "kind": kind, "name": "owner",
            "uid": "owner-uid", "controller": true,
        }]);
    }
    pod
}

fn with(mut pod: Value, patch: Value) -> Resource {
    for (pointer, value) in patch.as_object().expect("patch") {
        let (parent, key) = pointer.rsplit_once('/').expect("pointer");
        pod.pointer_mut(parent).expect("parent")[key] = value.clone();
    }
    object(pod)
}

fn evicted(plan: &DrainPlan) -> Vec<&str> {
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

/// One generated pod: its owner, whether it is a mirror pod, its phase and whether it mounts an
/// `emptyDir`.
fn generated(
    i: usize,
    owner: Option<&str>,
    mirror: bool,
    phase: &str,
    empty_dir: bool,
) -> Resource {
    let mut pod = pod_json(&format!("p{i}"), "n1", owner);
    pod["status"]["phase"] = json!(phase);
    if mirror {
        pod["metadata"]["annotations"] = json!({"kubernetes.io/config.mirror": "hash"});
    }
    if empty_dir {
        pod["spec"]["volumes"] = json!([{"name": "scratch", "emptyDir": {}}]);
    }
    object(pod)
}

proptest::proptest! {
    /// Whatever the pods and the flags, a DaemonSet pod is never evicted when DaemonSets are
    /// ignored, a mirror pod never is, every pod lands in exactly one list, and a blocked plan is
    /// one the flags could have unblocked.
    #[test]
    fn the_filter_never_evicts_daemonset_or_mirror_pods_when_told_to_skip(
        pods in proptest::collection::vec(
            (
                proptest::option::of(proptest::sample::select(vec![
                    "ReplicaSet", "DaemonSet", "StatefulSet", "Job",
                ])),
                proptest::bool::ANY,
                proptest::sample::select(vec!["Pending", "Running", "Succeeded", "Failed"]),
                proptest::bool::ANY,
            ),
            0..24,
        ),
        delete_emptydir_data in proptest::bool::ANY,
        force in proptest::bool::ANY,
    ) {
        let pods: Vec<Resource> = pods
            .iter()
            .enumerate()
            .map(|(i, (owner, mirror, phase, empty))| generated(i, *owner, *mirror, phase, *empty))
            .collect();
        let options = DrainOptions {
            ignore_daemonsets: true,
            delete_emptydir_data,
            force,
            ..DrainOptions::default()
        };
        let plan = plan_drain(&pods, &options);
        proptest::prop_assert_eq!(
            plan.evict.len() + plan.skipped.len() + plan.blocked.len(),
            pods.len()
        );
        for pod in &pods {
            let name = pod.name();
            let evicted = plan.evict.iter().any(|p| p.name == name);
            let mirror = pod.meta.annotations.contains_key("kubernetes.io/config.mirror");
            let daemon = pod
                .meta
                .controller_ref()
                .is_some_and(|owner| &*owner.kind == "DaemonSet");
            proptest::prop_assert!(!(evicted && (mirror || daemon)), "{name} was evicted");
        }
        // With DaemonSets ignored, a DaemonSet pod never blocks; with every flag on, nothing does.
        proptest::prop_assert!(plan.blocked.iter().all(|b| b.reason != BlockReason::DaemonSet));
        if delete_emptydir_data && force {
            proptest::prop_assert!(plan.blocked.is_empty());
        }
    }
}
