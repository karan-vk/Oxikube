//! `RolloutService::history`: the revisions a dialog shows.

use oxikube_domain::ErrorKind;
use oxikube_domain::rollout::RolloutKind;
use oxikube_testkit::ResourceCall;

use super::{Rollouts, deployment_ref, replicaset_ref, statefulset_ref};
use crate::rollout::RolloutService;

fn history(
    t: &Rollouts,
    target: &oxikube_domain::ids::ResourceRef,
) -> oxikube_domain::OxiResult<crate::rollout::RolloutHistory> {
    futures::executor::block_on(RolloutService::new(t.h.manager.clone()).history(target))
}

#[test]
fn a_deployments_history_is_newest_first_with_change_causes_and_the_current_marked() {
    let t = Rollouts::new(false);
    let found = history(&t, &deployment_ref()).unwrap();
    assert_eq!(found.kind, RolloutKind::Deployment);
    assert_eq!(found.workload, deployment_ref());
    let rows: Vec<_> = found
        .revisions
        .iter()
        .map(|r| {
            (
                r.revision,
                r.current,
                r.change_cause.clone(),
                r.images.clone(),
            )
        })
        .collect();
    assert_eq!(
        rows,
        [
            (
                3,
                true,
                Some("bump to v3".to_owned()),
                vec!["web:v3".to_owned()]
            ),
            (
                2,
                false,
                Some("bump to v2".to_owned()),
                vec!["web:v2".to_owned()]
            ),
            (1, false, None, vec!["web:v1".to_owned()]),
        ]
    );
    assert_eq!(found.current().unwrap().revision, 3);
    assert_eq!(
        found.preselected,
        Some(2),
        "the previous revision is the default"
    );
    assert!(!found.paused);
}

#[test]
fn reading_the_history_writes_nothing_and_starts_no_audit_record() {
    let t = Rollouts::new(false);
    history(&t, &deployment_ref()).unwrap();
    assert!(t.port().mutating_calls().is_empty());
    assert!(t.h.audit().is_empty());
    // Only reads: the Deployment and a list of ReplicaSets, filtered by its selector.
    let calls = t.port().recorded_calls();
    assert!(
        calls
            .iter()
            .any(|call| matches!(call, ResourceCall::List { options, .. }
            if options.label_selector.as_deref() == Some("app=web"))),
        "{calls:?}"
    );
}

#[test]
fn a_statefulsets_history_is_its_controller_revisions() {
    let t = Rollouts::new(false);
    let found = history(&t, &statefulset_ref()).unwrap();
    assert_eq!(found.kind, RolloutKind::StatefulSet);
    assert_eq!(
        found
            .revisions
            .iter()
            .map(|r| r.revision)
            .collect::<Vec<_>>(),
        [3, 2, 1]
    );
    assert!(found.revisions[0].current);
    assert_eq!(found.revisions[1].images, ["db:v2"]);
    assert_eq!(found.revisions[1].replicas, None);
}

#[test]
fn a_replicaset_shows_its_deployments_history_starting_on_its_own_revision() {
    let t = Rollouts::new(false);
    let found = history(&t, &replicaset_ref("web-1")).unwrap();
    assert_eq!(found.workload, deployment_ref());
    assert_eq!(found.preselected, Some(1));
    assert_eq!(found.revisions.len(), 3);
}

#[test]
fn a_paused_deployment_says_so() {
    let t = Rollouts::with(false, super::deployment_object(3, "web:v3", true));
    assert!(history(&t, &deployment_ref()).unwrap().paused);
}

#[test]
fn kinds_without_a_history_and_unknown_things_are_refused() {
    let t = Rollouts::new(false);
    let service = RolloutService::new(t.h.manager.clone());
    let pod = oxikube_domain::ids::ResourceRef::namespaced(
        crate::testing::id("a"),
        oxikube_domain::ids::Gvk::new("", "v1", "Pod"),
        "shop",
        "p",
    );
    let error = futures::executor::block_on(service.history(&pod)).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Validation);
    let missing = oxikube_domain::ids::ResourceRef::namespaced(
        crate::testing::id("a"),
        oxikube_domain::ids::Gvk::new("apps", "v1", "Deployment"),
        "shop",
        "ghost",
    );
    let error = futures::executor::block_on(service.history(&missing)).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::NotFound);
    // A ReplicaSet no Deployment owns has no history.
    t.port().insert(
        oxikube_domain::Resource::from_json(serde_json::json!({
            "apiVersion": "apps/v1", "kind": "ReplicaSet",
            "metadata": {"name": "loose", "namespace": "shop"},
            "spec": {}
        }))
        .unwrap(),
    );
    let error = futures::executor::block_on(service.history(&replicaset_ref("loose"))).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Validation);
    assert!(
        error.message().contains("not owned by a Deployment"),
        "{}",
        error.message()
    );
    let other = oxikube_domain::ids::ResourceRef::namespaced(
        crate::testing::id("b"),
        oxikube_domain::ids::Gvk::new("apps", "v1", "Deployment"),
        "shop",
        "web",
    );
    assert_eq!(
        futures::executor::block_on(service.history(&other))
            .unwrap_err()
            .kind(),
        ErrorKind::NotFound,
        "cluster b is not open"
    );
}
