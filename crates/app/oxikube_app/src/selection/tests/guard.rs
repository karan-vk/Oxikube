//! The guard around a bulk command: the tier is the highest among the objects, read-only blocks
//! the whole batch before anything is sent, and nothing runs before the confirmation.

use oxikube_domain::ErrorKind;
use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::{Command, Propagation};
use oxikube_domain::safety::{ConfirmTier, Risk};
use oxikube_domain::selection::{LabelEdit, LabelField};

use super::*;
use crate::command_bus::DispatchError;
use crate::testing::ctx;

fn label_of(targets: Vec<ResourceRef>) -> Command {
    Command::SelectionLabel {
        targets,
        field: LabelField::Labels,
        edits: vec![LabelEdit::set("team", "data")],
    }
}

fn pods(n: usize) -> Vec<ResourceRef> {
    (0..n).map(|i| pod_ref(&format!("p{i}"))).collect()
}

#[test]
fn a_selection_of_ordinary_objects_takes_one_simple_confirmation_that_counts_them() {
    let t = Selected::new(false);
    let mut targets = pods(3);
    targets.push(ns_ref("ConfigMap", "", "kube-system", "settings"));
    let request = t.h.ask(delete_of(targets), ctx(Initiator::Ui));
    assert_eq!(request.tier, ConfirmTier::Simple);
    assert_eq!(request.risk, Some(Risk::Medium));
    assert_eq!(request.expected_name, None);
    // The text names counts by kind and namespace, never every object.
    assert!(
        request.summary.contains("Delete 4 objects"),
        "{}",
        request.summary
    );
    assert!(
        request.summary.contains("3 Pods in default"),
        "{}",
        request.summary
    );
    assert!(
        request.summary.contains("1 ConfigMap in kube-system"),
        "{}",
        request.summary
    );
    assert!(
        t.writes().is_empty(),
        "nothing is sent before the confirmation"
    );
}

#[test]
fn one_namespace_among_pods_makes_the_whole_batch_type_the_name() {
    let t = Selected::new(false);
    let mut targets = pods(5);
    targets.push(namespace_ref("dev"));
    let request = t.h.ask(delete_of(targets), ctx(Initiator::Ui));
    assert_eq!(request.tier, ConfirmTier::TypeName);
    assert_eq!(request.risk, Some(Risk::Irreversible));
    // A selection has no single name: the cluster's context name is the deterministic token.
    let context = t.h.manager.get(&id("a")).unwrap().context().to_string();
    assert_eq!(request.expected_name.as_deref(), Some(context.as_str()));
    assert!(
        request.summary.contains("1 Namespace"),
        "{}",
        request.summary
    );
}

#[test]
fn a_cascading_delete_of_ordinary_objects_types_the_name_too() {
    let t = Selected::new(false);
    let command = Command::SelectionDelete {
        targets: pods(2),
        propagation: Propagation::Foreground,
    };
    let request = t.h.ask(command, ctx(Initiator::Ui));
    assert_eq!(
        (request.tier, request.risk),
        (ConfirmTier::TypeName, Some(Risk::High))
    );
    assert!(request.summary.contains("Dependents are deleted first"));
}

#[test]
fn the_size_of_a_selection_never_lowers_or_raises_the_tier() {
    let t = Selected::new(false);
    for n in [1, 40, 2_000] {
        let request = t.h.ask(delete_of(pods(n)), ctx(Initiator::Ui));
        assert_eq!(request.tier, ConfirmTier::Simple, "{n} pods");
    }
    // And a namespace is high however many pods surround it, or none.
    for n in [0, 2_000] {
        let mut targets = pods(n);
        targets.push(namespace_ref("dev"));
        let request = t.h.ask(delete_of(targets), ctx(Initiator::Ui));
        assert_eq!(
            request.tier,
            ConfirmTier::TypeName,
            "{n} pods and a namespace"
        );
    }
}

#[test]
fn restarts_and_label_edits_take_one_simple_confirmation_each() {
    let t = Selected::new(false);
    let restart =
        t.h.ask(restart_of(vec![deployment_ref("web")]), ctx(Initiator::Ui));
    assert_eq!(
        (restart.tier, restart.risk),
        (ConfirmTier::Simple, Some(Risk::Low))
    );
    assert!(
        restart.summary.contains("Restart 1 object"),
        "{}",
        restart.summary
    );
    let label = t.h.ask(label_of(pods(2)), ctx(Initiator::Ui));
    assert_eq!(
        (label.tier, label.risk),
        (ConfirmTier::Simple, Some(Risk::Medium))
    );
    assert!(
        label.summary.contains("Change 1 and remove 0 label"),
        "{}",
        label.summary
    );
    // The text never carries a value.
    assert!(!label.summary.contains("data"), "{}", label.summary);
}

#[test]
fn a_wrong_typed_name_sends_nothing() {
    let t = Selected::new(false);
    t.port().insert(namespace("dev"));
    t.port().insert(pod("p0", "default", json!({})));
    let command = delete_of(vec![pod_ref("p0"), namespace_ref("dev")]);
    let request = t.h.ask(command.clone(), ctx(Initiator::Ui));
    let answer = crate::guard::Confirmation::typed(request.token, "not-the-context");
    let denied =
        t.h.dispatch(command, ctx(Initiator::Ui).with_confirmation(answer));
    assert!(
        matches!(denied, Err(DispatchError::Confirmation(_))),
        "{denied:?}"
    );
    // A namespace makes the batch irreversible: the guard dry-ran it before asking, and that is
    // all that was sent.
    let Some(crate::guard::DryRunPreview::Rehearsed(preview)) = &request.preview else {
        panic!("the batch is dry-run before it is asked: {request:?}");
    };
    assert!(preview.message.is_some(), "the dry run's report");
    assert!(
        preview.changes.is_empty(),
        "a delete writes no object to compare"
    );
    assert!(
        t.writes().iter().all(|(_, _, dry_run)| *dry_run),
        "{:?}",
        t.writes()
    );
    assert_eq!(t.port().objects().len(), 2);
}

#[test]
fn read_only_blocks_the_whole_batch_for_every_initiator_before_any_request() {
    let t = Selected::new(true);
    t.port().insert(pod("p0", "default", json!({})));
    t.port().insert(deployment("web"));
    let commands = [
        delete_of(vec![pod_ref("p0")]),
        restart_of(vec![deployment_ref("web")]),
        label_of(vec![pod_ref("p0")]),
    ];
    for command in commands {
        for initiator in [Initiator::Ui, Initiator::Agent] {
            let result = t.h.dispatch(command.clone(), ctx(initiator));
            assert!(
                matches!(result, Err(DispatchError::ReadOnly { .. })),
                "{} as {initiator:?}: {result:?}",
                command.id()
            );
        }
    }
    assert!(
        t.port().recorded_calls().is_empty(),
        "no request of any kind was sent"
    );
    let denied = t.h.audit();
    assert_eq!(denied.len(), 6);
    assert!(denied.iter().all(|r| r.outcome == AuditOutcome::Denied));
    // The flow reports it as a refusal of the batch, not as failed rows.
    let flow = t.flow();
    let plan = flow.plan(delete_of(vec![pod_ref("p0")])).unwrap();
    let error = futures::executor::block_on(flow.run(&plan, None)).unwrap_err();
    match error {
        crate::selection::SelectionError::Refused(error) => {
            assert_eq!(error.kind(), ErrorKind::Forbidden)
        }
        other => panic!("{other:?}"),
    }
}
