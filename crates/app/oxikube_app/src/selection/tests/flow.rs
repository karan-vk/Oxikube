//! `SelectionFlow`: the plan a dialog draws, the typed phrase, and a retry of the failed rows.

use oxikube_domain::OxiError;
use oxikube_domain::safety::{ConfirmTier, Risk};

use super::*;
use crate::selection::SelectionError;

fn with_pods(names: &[&str]) -> Selected {
    let t = Selected::new(false);
    for name in names {
        t.port().insert(pod(name, "default", json!({})));
    }
    t
}

#[test]
fn the_plan_is_the_guards_own_tier_and_names_what_is_typed() {
    let t = Selected::new(false);
    let flow = t.flow();
    let plain = flow
        .plan(delete_of(vec![pod_ref("a"), pod_ref("b")]))
        .unwrap();
    assert_eq!(
        (plain.tier(), plain.risk(), plain.phrase()),
        (ConfirmTier::Simple, Risk::Medium, None)
    );
    assert_eq!(plain.summary().total(), 2);

    let with_namespace = flow
        .plan(delete_of(vec![pod_ref("a"), namespace_ref("dev")]))
        .unwrap();
    assert_eq!(with_namespace.tier(), ConfirmTier::TypeName);
    assert_eq!(with_namespace.phrase(), Some(with_namespace.context()));
    assert_eq!(with_namespace.risk(), Risk::Irreversible);
}

#[test]
fn nothing_selected_and_two_clusters_cannot_be_planned() {
    let t = Selected::new(false);
    t.h.connect("b", false);
    let flow = t.flow();
    assert!(matches!(
        flow.plan(delete_of(Vec::new())),
        Err(SelectionError::Empty)
    ));
    let other = ResourceRef::namespaced(id("b"), Gvk::new("", "v1", "Pod"), "default", "x");
    assert!(matches!(
        flow.plan(delete_of(vec![pod_ref("a"), other])),
        Err(SelectionError::MixedClusters)
    ));
}

#[test]
fn a_typed_phrase_that_does_not_match_sends_nothing() {
    let t = with_pods(&["a"]);
    t.port().insert(namespace("dev"));
    let flow = t.flow();
    let plan = flow
        .plan(delete_of(vec![pod_ref("a"), namespace_ref("dev")]))
        .unwrap();
    for typed in [None, Some("nope")] {
        let error = futures::executor::block_on(flow.run(&plan, typed)).unwrap_err();
        assert!(
            matches!(error, SelectionError::NameMismatch { .. }),
            "{error:?}"
        );
    }
    assert!(t.writes().is_empty());
    let report = futures::executor::block_on(flow.run(&plan, plan.phrase())).expect("typed");
    assert_eq!(report.succeeded(), 2);
    assert!(t.port().objects().is_empty());
}

#[test]
fn failed_rows_can_be_retried_on_their_own() {
    let t = with_pods(&["a", "b", "c"]);
    t.port()
        .script()
        .delete
        .push_err(OxiError::forbidden("not now"));
    let flow = t.flow();
    let plan = flow
        .plan(delete_of(vec![pod_ref("a"), pod_ref("b"), pod_ref("c")]))
        .unwrap();
    let first = futures::executor::block_on(flow.run(&plan, None)).unwrap();
    assert_eq!((first.succeeded(), first.failed()), (2, 1));

    let retry = flow.plan_again(&plan, first.failed_targets()).unwrap();
    assert_eq!(retry.targets(), [pod_ref("a")]);
    let second = futures::executor::block_on(flow.run(&retry, None)).unwrap();
    assert_eq!((second.succeeded(), second.failed()), (1, 0));
    assert_ne!(first.batch, second.batch, "a retry is a batch of its own");
    assert!(t.port().objects().is_empty());
}

#[test]
fn a_restart_plan_over_a_pod_is_refused_when_run() {
    let t = with_pods(&["a"]);
    let flow = t.flow();
    let plan = flow.plan(restart_of(vec![pod_ref("a")])).unwrap();
    let error = futures::executor::block_on(flow.run(&plan, None)).unwrap_err();
    match error {
        SelectionError::Refused(error) => {
            assert_eq!(error.kind(), oxikube_domain::ErrorKind::Validation)
        }
        other => panic!("{other:?}"),
    }
}
