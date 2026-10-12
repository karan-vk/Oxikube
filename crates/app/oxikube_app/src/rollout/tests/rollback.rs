//! `workload::Rollback` end to end over the guard: what is patched, with what, and the refusals.

use oxikube_domain::ErrorKind;
use oxikube_domain::audit::Initiator;
use oxikube_ports::PatchKind;
use serde_json::json;

use super::{
    Rollouts, deployment_object, deployment_ref, replicaset_ref, rollback, statefulset_ref,
};
use crate::command_bus::DispatchError;
use crate::testing::ctx;

fn handler_error(error: DispatchError) -> oxikube_domain::OxiError {
    match error {
        DispatchError::Handler(error) => error,
        other => panic!("expected a handler error, got {other:?}"),
    }
}

#[test]
fn undo_to_the_previous_revision_replaces_the_template_without_the_hash_label() {
    let t = Rollouts::new(false);
    t.answer_patches();
    let output =
        t.h.confirm_and_run(rollback(deployment_ref(), None), ctx(Initiator::Ui))
            .unwrap()
            .completed()
            .unwrap();
    assert_eq!(
        output.message.as_deref(),
        Some("Rolled back Deployment shop/web from revision 3 to revision 2")
    );
    let data = output.data.unwrap();
    assert_eq!(
        (data["from_revision"].as_i64(), data["to_revision"].as_i64()),
        (Some(3), Some(2))
    );
    assert_eq!(data["outcome"], "rolled_back");

    // The server rehearses the patch, then the same patch is sent for real.
    let patches = t.patches();
    assert_eq!(patches.len(), 2);
    assert_eq!((patches[0].0, patches[1].0), (true, false));
    assert_eq!(patches[0].2, patches[1].2);
    let (_, kind, body) = &patches[1];
    assert_eq!(*kind, PatchKind::Json);
    assert_eq!(body[0]["op"], "replace");
    assert_eq!(body[0]["path"], "/spec/template");
    assert_eq!(body[0]["value"]["spec"]["containers"][0]["image"], "web:v2");
    assert!(
        body[0]["value"]["metadata"]["labels"]
            .get("pod-template-hash")
            .is_none()
    );
    assert_eq!(
        body[1]["value"]["kubernetes.io/change-cause"], "bump to v2",
        "the cause comes back with the template"
    );
}

#[test]
fn undo_to_a_given_revision_restores_that_one() {
    let t = Rollouts::new(false);
    t.answer_patches();
    let output =
        t.h.confirm_and_run(rollback(deployment_ref(), Some(1)), ctx(Initiator::Ui))
            .unwrap()
            .completed()
            .unwrap();
    assert_eq!(output.data.unwrap()["to_revision"], 1);
    assert_eq!(
        t.patches()[1].2[0]["value"]["spec"]["containers"][0]["image"],
        "web:v1"
    );
}

#[test]
fn a_replicaset_rolls_its_deployment_back_to_its_own_revision() {
    let t = Rollouts::new(false);
    t.answer_patches();
    let output =
        t.h.confirm_and_run(rollback(replicaset_ref("web-1"), None), ctx(Initiator::Ui))
            .unwrap()
            .completed()
            .unwrap();
    assert_eq!(
        output.message.as_deref(),
        Some("Rolled back Deployment shop/web from revision 3 to revision 1")
    );
    let calls = t.port().mutating_calls();
    assert!(matches!(
        &calls[1],
        oxikube_testkit::ResourceCall::Patch { kind, name, .. }
            if kind.kind.as_ref() == "Deployment" && name == "web"
    ));
}

#[test]
fn the_target_being_current_is_reported_and_nothing_is_sent() {
    let t = Rollouts::new(false);
    let output =
        t.h.confirm_and_run(rollback(deployment_ref(), Some(3)), ctx(Initiator::Ui))
            .unwrap()
            .completed()
            .unwrap();
    assert_eq!(
        output.message.as_deref(),
        Some("Deployment shop/web is already at revision 3")
    );
    assert_eq!(output.data.unwrap()["outcome"], "already_current");
    assert!(t.port().mutating_calls().is_empty());
}

#[test]
fn a_paused_deployment_is_refused_with_kubectls_message() {
    let t = Rollouts::with(false, deployment_object(3, "web:v3", true));
    let error = handler_error(
        t.h.confirm_and_run(rollback(deployment_ref(), None), ctx(Initiator::Ui))
            .unwrap_err(),
    );
    assert_eq!(error.kind(), ErrorKind::Conflict);
    assert!(
        t.port()
            .recorded_calls()
            .iter()
            .all(|call| !matches!(call, oxikube_testkit::ResourceCall::List { .. })),
        "refused after one read, before the ReplicaSets are listed"
    );
    assert!(
        error
            .message()
            .starts_with("you cannot rollback a paused deployment; resume it first"),
        "{}",
        error.message()
    );
    assert!(t.port().mutating_calls().is_empty());
}

#[test]
fn a_pruned_revision_is_refused_before_anything_is_sent() {
    let t = Rollouts::new(false);
    let error = handler_error(
        t.h.confirm_and_run(rollback(deployment_ref(), Some(9)), ctx(Initiator::Ui))
            .unwrap_err(),
    );
    assert_eq!(error.kind(), ErrorKind::Validation);
    assert!(
        error.message().contains("unable to find revision 9"),
        "{}",
        error.message()
    );
    assert!(t.port().mutating_calls().is_empty());
}

#[test]
fn a_statefulset_undo_patches_with_the_controller_revisions_data_as_a_strategic_merge() {
    let t = Rollouts::new(false);
    t.answer_patches();
    let output =
        t.h.confirm_and_run(rollback(statefulset_ref(), Some(1)), ctx(Initiator::Ui))
            .unwrap()
            .completed()
            .unwrap();
    assert_eq!(
        output.message.as_deref(),
        Some("Rolled back StatefulSet shop/db from revision 3 to revision 1")
    );
    let (_, kind, body) = &t.patches()[1];
    assert_eq!(*kind, PatchKind::Strategic);
    assert_eq!(
        *body,
        json!({"spec": {"template": {
            "$patch": "replace",
            "metadata": {"labels": {"app": "db"}, "creationTimestamp": null},
            "spec": {"containers": [{"name": "db", "image": "db:v1"}]},
        }}})
    );
}

#[test]
fn a_statefulset_already_on_the_target_template_sends_nothing() {
    let t = Rollouts::new(false);
    let output =
        t.h.confirm_and_run(rollback(statefulset_ref(), Some(3)), ctx(Initiator::Ui))
            .unwrap()
            .completed()
            .unwrap();
    assert_eq!(output.data.unwrap()["outcome"], "already_current");
    assert!(t.port().mutating_calls().is_empty());
}

#[test]
fn a_dry_run_dispatch_has_the_server_rehearse_once_and_changes_nothing() {
    let t = Rollouts::new(false);
    t.port()
        .script()
        .patch
        .push_ok(deployment_object(4, "x", false));
    let output =
        t.h.confirm_and_run(
            rollback(deployment_ref(), None),
            ctx(Initiator::Agent).with_dry_run(true),
        )
        .unwrap()
        .completed()
        .unwrap();
    assert_eq!(
        output.message.as_deref(),
        Some("Dry run: would roll back Deployment shop/web from revision 3 to revision 2")
    );
    let patches = t.patches();
    assert_eq!(patches.len(), 1);
    assert!(patches[0].0, "dry run");
}

#[test]
fn a_kind_without_a_history_is_refused() {
    let t = Rollouts::new(false);
    let pod = oxikube_domain::ids::ResourceRef::namespaced(
        crate::testing::id("a"),
        oxikube_domain::ids::Gvk::new("", "v1", "Pod"),
        "shop",
        "p",
    );
    let error = handler_error(
        t.h.confirm_and_run(rollback(pod, None), ctx(Initiator::Ui))
            .unwrap_err(),
    );
    assert_eq!(error.kind(), ErrorKind::Validation);
}
