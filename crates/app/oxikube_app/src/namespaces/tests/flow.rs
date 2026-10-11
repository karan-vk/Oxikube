//! The generic delete flow (plan, type the name, results) driving `namespace::Delete`.

use std::collections::{BTreeMap, HashMap};

use oxikube_domain::audit::AuditOutcome;
use oxikube_domain::command::Propagation;
use oxikube_domain::safety::{ConfirmTier, Risk};

use super::{harness, namespace_ref};
use crate::actions::{DeleteError, DeleteFlow, ItemStatus};
use crate::namespaces::delete_command;

fn flow(h: &crate::testing::Harness, contents: &[(&str, &[(&str, u32)])]) -> DeleteFlow {
    let contents: HashMap<String, BTreeMap<String, u32>> = contents
        .iter()
        .map(|(name, held)| {
            (
                (*name).to_owned(),
                held.iter().map(|(k, n)| ((*k).to_owned(), *n)).collect(),
            )
        })
        .collect();
    DeleteFlow::new(h.bus.clone(), h.manager.clone(), "alice")
        .with_command(delete_command(contents))
}

#[test]
fn the_plan_asks_what_the_namespace_guard_asks() {
    let h = harness();
    h.connect("a", false);
    let plan = flow(&h, &[])
        .plan(&[namespace_ref("a", "payments")], Propagation::Background)
        .unwrap();
    assert_eq!(plan.tier(), ConfirmTier::TypeName);
    assert_eq!(plan.risk(), Risk::Irreversible);
    assert_eq!(plan.phrase(), Some("payments"));
    assert_eq!(plan.items()[0].command.id().as_str(), "namespace::Delete");
}

#[test]
fn running_it_needs_the_typed_name_and_audits_the_counts() {
    let h = harness();
    h.connect("a", false);
    h.allow_deletes("a", 2);
    let flow = flow(&h, &[("payments", &[("Pod", 4)])]);
    let plan = flow
        .plan(&[namespace_ref("a", "payments")], Propagation::Background)
        .unwrap();

    let err = futures::executor::block_on(flow.run(&plan, Some("pay"))).unwrap_err();
    assert!(matches!(err, DeleteError::NameMismatch { .. }));
    assert!(h.writes("a").is_empty());

    let report = futures::executor::block_on(flow.run(&plan, Some("payments"))).unwrap();
    assert_eq!(report.items[0].status, ItemStatus::Deleted);
    let audit = h.audit();
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0].outcome, AuditOutcome::Succeeded);
    assert_eq!(&*audit[0].cmd, "namespace::Delete");
    assert_eq!(audit[0].detail.as_deref(), Some("contents=Pod:4"));
}

#[test]
fn read_only_is_a_forbidden_result_not_a_delete() {
    let h = harness();
    h.connect("a", true);
    let flow = flow(&h, &[]);
    let plan = flow
        .plan(&[namespace_ref("a", "payments")], Propagation::Background)
        .unwrap();
    let report = futures::executor::block_on(flow.run(&plan, Some("payments"))).unwrap();
    assert!(matches!(
        report.items[0].status,
        ItemStatus::Failed(_) | ItemStatus::Forbidden(_)
    ));
    assert!(h.writes("a").is_empty());
}
