//! A bulk command works on a few objects at a time: it neither floods the API server with the whole
//! selection nor goes one by one. The fake port records how many writes were in flight at once.

use oxikube_domain::OxiError;
use oxikube_domain::audit::Initiator;
use oxikube_domain::selection::{LabelEdit, LabelField};

use super::*;
use crate::selection::BulkReport;
use crate::workloads::CONCURRENCY;

const PODS: usize = CONCURRENCY * 4;

fn many_pods() -> (Selected, Vec<ResourceRef>) {
    let t = Selected::new(false);
    let names: Vec<String> = (0..PODS).map(|i| format!("p{i}")).collect();
    for name in &names {
        t.port().insert(pod(name, "default", json!({})));
    }
    let targets = names.iter().map(|n| pod_ref(n)).collect();
    (t, targets)
}

#[test]
fn a_bulk_delete_has_exactly_the_concurrency_limit_in_flight() {
    let (t, targets) = many_pods();
    let out = t
        .run_overlapping(delete_of(targets), Initiator::Ui)
        .expect("delete");
    let report = BulkReport::from_data(out.data.as_ref()).expect("a report");
    assert_eq!(report.succeeded(), PODS);
    assert!(t.port().objects().is_empty());
    assert_eq!(
        t.port().peak_writes(),
        CONCURRENCY,
        "not one at a time, not the whole selection at once"
    );
}

#[test]
fn a_bulk_label_edit_has_exactly_the_concurrency_limit_in_flight() {
    let (t, targets) = many_pods();
    let command = Command::SelectionLabel {
        targets,
        field: LabelField::Labels,
        edits: vec![LabelEdit::set("team", "data")],
    };
    let out = t.run_overlapping(command, Initiator::Ui).expect("label");
    let report = BulkReport::from_data(out.data.as_ref()).expect("a report");
    assert_eq!(report.succeeded(), PODS);
    assert_eq!(t.port().peak_writes(), CONCURRENCY);
}

#[test]
fn a_row_whose_audit_record_cannot_be_written_sends_nothing() {
    let t = Selected::new(false);
    let names = ["a", "b", "c"];
    for name in names {
        t.port().insert(pod(name, "default", json!({})));
    }
    // The audit store fails from the first row's flush on: the object of that row was touched,
    // and every later row has to stop at its own precondition (fail closed).
    for _ in 0..10 {
        t.h.state
            .script()
            .append_audit
            .push_err(OxiError::internal("disk full"));
    }
    let targets = names.iter().map(|n| pod_ref(n)).collect();
    // The command itself reports the log as failed, so there is no report to read: the port is
    // the witness.
    let _ = t.run(delete_of(targets), Initiator::Ui);
    let touched: std::collections::BTreeSet<String> =
        t.writes().into_iter().map(|(_, name, _)| name).collect();
    assert_eq!(
        touched.len(),
        1,
        "only the row before the failure was sent: {touched:?}"
    );
    assert_eq!(t.port().objects().len(), 2, "the other pods are untouched");
}
