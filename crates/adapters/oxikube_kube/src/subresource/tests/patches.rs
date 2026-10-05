//! The kdash-derived builders produce exact JSON.

use jiff::Timestamp;
use oxikube_ports::PatchKind;
use serde_json::json;

use crate::subresource::{RESTARTED_AT_ANNOTATION, ResourcePatch};

fn at(text: &str) -> Timestamp {
    text.parse().expect("timestamp")
}

#[test]
fn scale_sets_spec_replicas() {
    assert_eq!(
        ResourcePatch::Scale(3).to_merge_patch(),
        json!({"spec": {"replicas": 3}})
    );
    assert_eq!(
        ResourcePatch::Scale(0).to_merge_patch(),
        json!({"spec": {"replicas": 0}})
    );
}

#[test]
fn rollout_restart_stamps_the_pod_template_in_whole_second_utc() {
    let patch = ResourcePatch::RolloutRestart {
        at: at("2026-10-06T12:34:56.789123Z"),
    };
    assert_eq!(
        patch.to_merge_patch(),
        json!({"spec": {"template": {"metadata": {"annotations": {
            "kubectl.kubernetes.io/restartedAt": "2026-10-06T12:34:56Z"
        }}}}})
    );
    assert_eq!(RESTARTED_AT_ANNOTATION, "kubectl.kubernetes.io/restartedAt");
}

#[test]
fn a_later_restart_changes_the_body() {
    let first = ResourcePatch::RolloutRestart {
        at: at("2026-10-06T12:00:00Z"),
    };
    let second = ResourcePatch::RolloutRestart {
        at: at("2026-10-06T12:00:01Z"),
    };
    assert_ne!(first.to_merge_patch(), second.to_merge_patch());
}

#[test]
fn cordon_and_uncordon_flip_unschedulable() {
    assert_eq!(
        ResourcePatch::Cordon.to_merge_patch(),
        json!({"spec": {"unschedulable": true}})
    );
    assert_eq!(
        ResourcePatch::Uncordon.to_merge_patch(),
        json!({"spec": {"unschedulable": false}})
    );
}

#[test]
fn suspend_and_resume_set_spec_suspend() {
    assert_eq!(
        ResourcePatch::CronJobSuspend(true).to_merge_patch(),
        json!({"spec": {"suspend": true}})
    );
    assert_eq!(
        ResourcePatch::CronJobSuspend(false).to_merge_patch(),
        json!({"spec": {"suspend": false}})
    );
}

#[test]
fn every_builder_is_a_merge_patch_with_the_same_body() {
    let all = [
        ResourcePatch::Scale(2),
        ResourcePatch::RolloutRestart {
            at: at("2026-10-06T12:00:00Z"),
        },
        ResourcePatch::Cordon,
        ResourcePatch::Uncordon,
        ResourcePatch::CronJobSuspend(true),
    ];
    for builder in all {
        let patch = builder.to_patch();
        assert_eq!(patch.kind, PatchKind::Merge, "{builder:?}");
        assert_eq!(patch.body, builder.to_merge_patch(), "{builder:?}");
    }
}
