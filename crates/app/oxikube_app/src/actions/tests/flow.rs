//! `DeleteFlow`: planning, and deleting a selection with per-object results.

use futures::FutureExt as _;
use oxikube_domain::ErrorKind;
use oxikube_domain::OxiError;
use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::Propagation;
use oxikube_domain::ids::ResourceRef;
use oxikube_domain::safety::{ConfirmTier, Risk};
use oxikube_ports::DeleteOutcome;

use super::{cluster_scoped, namespaced};
use crate::actions::{DeleteError, DeleteFlow, DeleteReport, ItemStatus};
use crate::testing::Harness;

fn pod(name: &str) -> ResourceRef {
    namespaced("a", "", "Pod", name)
}

fn flow(h: &Harness) -> DeleteFlow {
    DeleteFlow::new(h.bus.clone(), h.manager.clone(), "alice").with_concurrency(1)
}

fn run(
    flow: &DeleteFlow,
    plan: &crate::actions::DeletePlan,
    typed: Option<&str>,
) -> Result<DeleteReport, DeleteError> {
    flow.run(plan, typed)
        .now_or_never()
        .expect("fakes answer at once")
}

#[test]
fn a_pod_plan_needs_one_click() {
    let h = Harness::with_delete_handler();
    h.connect("a", false);
    let plan = flow(&h)
        .plan(&[pod("web-0")], Propagation::Background)
        .unwrap();
    assert_eq!(plan.tier(), ConfirmTier::Simple);
    assert_eq!(plan.risk(), Risk::Medium);
    assert_eq!(plan.phrase(), None);
    assert_eq!(plan.kinds(), [("Pod".into(), 1)]);
    assert!(h.writes("a").is_empty(), "planning sends nothing");
    assert!(h.audit().is_empty());
}

#[test]
fn a_namespace_plan_asks_for_its_name() {
    let h = Harness::with_delete_handler();
    h.connect("a", false);
    let plan = flow(&h)
        .plan(
            &[cluster_scoped("a", "Namespace", "payments")],
            Propagation::Background,
        )
        .unwrap();
    assert_eq!(plan.tier(), ConfirmTier::TypeName);
    assert_eq!(plan.phrase(), Some("payments"));
    assert_eq!(plan.risk(), Risk::Irreversible);
}

#[test]
fn a_selection_with_a_namespace_asks_for_the_cluster_name_once() {
    let h = Harness::with_delete_handler();
    h.connect("a", false);
    let plan = flow(&h)
        .plan(
            &[
                pod("web-0"),
                pod("web-1"),
                cluster_scoped("a", "Namespace", "payments"),
            ],
            Propagation::Background,
        )
        .unwrap();
    assert_eq!(plan.tier(), ConfirmTier::TypeName);
    assert_eq!(plan.phrase(), Some("a"), "the cluster's context name");
    assert_eq!(plan.kinds(), [("Namespace".into(), 1), ("Pod".into(), 2)]);
    let typed = plan
        .items()
        .iter()
        .filter(|i| i.tier == ConfirmTier::TypeName);
    assert_eq!(typed.count(), 1);
    assert_eq!(plan.items().len(), 3);
}

#[test]
fn plans_reject_nothing_selected_and_mixed_clusters() {
    let h = Harness::with_delete_handler();
    h.connect("a", false);
    let flow = flow(&h);
    assert_eq!(
        flow.plan(&[], Propagation::Background).unwrap_err(),
        DeleteError::Empty
    );
    assert_eq!(
        flow.plan(
            &[pod("web-0"), namespaced("b", "", "Pod", "web-0")],
            Propagation::Background
        )
        .unwrap_err(),
        DeleteError::MixedClusters
    );
}

#[test]
fn a_wrong_phrase_sends_nothing() {
    let h = Harness::with_delete_handler();
    h.connect("a", false);
    let flow = flow(&h);
    let plan = flow
        .plan(
            &[cluster_scoped("a", "Namespace", "payments")],
            Propagation::Background,
        )
        .unwrap();
    for typed in [None, Some(""), Some("payment"), Some("Payments")] {
        let err = run(&flow, &plan, typed).unwrap_err();
        assert_eq!(
            err,
            DeleteError::NameMismatch {
                expected: "payments".into()
            }
        );
    }
    assert!(h.writes("a").is_empty());
    assert!(h.audit().is_empty());
    h.allow_deletes("a", 2);
    let report = run(&flow, &plan, Some("payments")).unwrap();
    assert_eq!(report.succeeded(), 1);
    assert_eq!(h.writes("a").len(), 2);
}

#[test]
fn bulk_delete_reports_each_object_and_audits_each() {
    let h = Harness::with_delete_handler();
    h.connect("a", false);
    let resources = h.resources("a");
    // Two requests per object (the server rehearsal, then the delete), in selection order.
    let ok = || {
        resources.script().delete.push_ok(DeleteOutcome::Deleted);
        resources.script().delete.push_ok(DeleteOutcome::Deleted);
    };
    ok(); // web-0
    resources
        .script()
        .delete
        .push_err(OxiError::forbidden("pods \"web-1\" is forbidden")); // web-1
    ok(); // web-2
    resources
        .script()
        .delete
        .push_err(OxiError::not_found("pods \"web-3\" not found")); // web-3
    ok(); // web-4

    let flow = flow(&h);
    let targets: Vec<_> = (0..5).map(|i| pod(&format!("web-{i}"))).collect();
    let plan = flow.plan(&targets, Propagation::Background).unwrap();
    assert_eq!(plan.tier(), ConfirmTier::Simple);
    let report = run(&flow, &plan, None).unwrap();

    let statuses: Vec<_> = report.items.iter().map(|i| i.status.clone()).collect();
    assert_eq!(
        statuses,
        [
            ItemStatus::Deleted,
            ItemStatus::Forbidden("pods \"web-1\" is forbidden".into()),
            ItemStatus::Deleted,
            ItemStatus::NotFound("pods \"web-3\" not found".into()),
            ItemStatus::Deleted,
        ]
    );
    assert_eq!(
        report.items.iter().map(|i| &i.target).collect::<Vec<_>>(),
        targets.iter().collect::<Vec<_>>(),
        "results keep the selection's order"
    );
    assert_eq!((report.succeeded(), report.failed()), (3, 2));
    assert_eq!(report.summary(), "Deleted 3 of 5 objects, 2 failed");

    // One audit record per object, each from the UI.
    let audit = h.audit();
    assert_eq!(audit.len(), 5);
    assert!(audit.iter().all(|r| r.initiator == Initiator::Ui));
    assert!(audit.iter().all(|r| &*r.who == "alice"));
    let outcomes: Vec<_> = audit.iter().map(|r| r.outcome).collect();
    assert_eq!(
        outcomes,
        [
            AuditOutcome::Succeeded,
            AuditOutcome::Failed,
            AuditOutcome::Succeeded,
            AuditOutcome::Failed,
            AuditOutcome::Succeeded
        ]
    );
    assert_eq!(
        audit.iter().map(|r| r.target.clone()).collect::<Vec<_>>(),
        targets
    );
}

#[test]
fn a_selection_with_a_namespace_confirms_it_with_the_typed_cluster_name() {
    let h = Harness::with_delete_handler();
    h.connect("a", false);
    h.allow_deletes("a", 4);
    let flow = flow(&h);
    let plan = flow
        .plan(
            &[pod("web-0"), cluster_scoped("a", "Namespace", "payments")],
            Propagation::Background,
        )
        .unwrap();
    assert!(
        run(&flow, &plan, Some("payments")).is_err(),
        "the object's name is not the phrase"
    );
    assert!(h.writes("a").is_empty());
    let report = run(&flow, &plan, Some("a")).unwrap();
    assert_eq!(report.succeeded(), 2);
    assert_eq!(h.writes("a").len(), 4);
}

#[test]
fn a_read_only_cluster_refuses_each_object_and_audits_each_refusal() {
    let h = Harness::with_delete_handler();
    h.connect("a", true);
    let flow = flow(&h);
    let plan = flow
        .plan(&[pod("web-0"), pod("web-1")], Propagation::Background)
        .unwrap();
    let report = run(&flow, &plan, None).unwrap();
    assert!(
        report
            .items
            .iter()
            .all(|i| matches!(i.status, ItemStatus::Forbidden(_)))
    );
    assert!(h.writes("a").is_empty());
    assert_eq!(h.audit().len(), 2);
    assert!(h.audit().iter().all(|r| r.outcome == AuditOutcome::Denied));
}

#[test]
fn results_keep_the_selection_order_when_objects_run_together() {
    let h = Harness::with_delete_handler();
    h.connect("a", false);
    h.allow_deletes("a", 16);
    let flow = DeleteFlow::new(h.bus.clone(), h.manager, "alice");
    let targets: Vec<_> = (0..8).map(|i| pod(&format!("web-{i}"))).collect();
    let plan = flow.plan(&targets, Propagation::Foreground).unwrap();
    let report = run(&flow, &plan, Some("a")).unwrap();
    assert_eq!(
        report
            .items
            .iter()
            .map(|i| i.target.clone())
            .collect::<Vec<_>>(),
        targets
    );
    assert_eq!(report.summary(), "Deleted 8 objects");
}

#[test]
fn error_kinds_map_to_item_statuses() {
    let h = Harness::with_delete_handler();
    h.connect("a", false);
    h.resources("a").script().delete.push_err(OxiError::new(
        ErrorKind::Conflict,
        "the object was modified",
    ));
    let flow = flow(&h);
    let plan = flow.plan(&[pod("web-0")], Propagation::Background).unwrap();
    let report = run(&flow, &plan, None).unwrap();
    assert_eq!(
        report.items[0].status,
        ItemStatus::Failed("the object was modified".into())
    );
    assert_eq!(report.items[0].status.label(), "Failed");
    assert_eq!(report.summary(), "Deleted 0 of 1 object, 1 failed");
}
