//! Running a bulk delete: per-row results, partial failure, audit per object under a batch id,
//! dry run and the limits of a batch.

use oxikube_domain::OxiError;
use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_ports::DeleteOutcome;

use super::*;
use crate::command_bus::DispatchContext;
use crate::selection::{BulkReport, MAX_BATCH};

fn report(out: &CommandOutput) -> BulkReport {
    BulkReport::from_data(out.data.as_ref()).expect("a report")
}

fn with_pods(names: &[&str]) -> Selected {
    let t = Selected::new(false);
    for name in names {
        t.port().insert(pod(name, "default", json!({})));
    }
    t
}

#[test]
fn every_selected_object_is_deleted_and_gets_its_own_row() {
    let t = with_pods(&["a", "b", "c"]);
    t.port().insert(
        Resource::from_json(json!({
            "apiVersion": "v1", "kind": "ConfigMap",
            "metadata": {"name": "settings", "namespace": "default"},
        }))
        .unwrap(),
    );
    let targets = vec![
        pod_ref("a"),
        ns_ref("ConfigMap", "", "default", "settings"),
        pod_ref("b"),
        pod_ref("c"),
    ];
    let out = t
        .run(delete_of(targets.clone()), Initiator::Ui)
        .expect("delete");
    let report = report(&out);
    assert_eq!(out.message.as_deref(), Some("Deleted 4 objects"));
    assert_eq!(
        report
            .rows
            .iter()
            .map(|r| r.target.clone())
            .collect::<Vec<_>>(),
        targets
    );
    assert!(report.rows.iter().all(|r| r.status.is_ok()));
    assert!(t.port().objects().is_empty());
}

#[test]
fn a_gone_object_is_skipped_and_a_refused_one_fails_but_the_others_continue() {
    let t = with_pods(&["a", "b", "c", "d"]);
    // The first rehearsal is refused (RBAC); `ghost` was never there.
    t.port()
        .script()
        .delete
        .push_err(OxiError::forbidden("pods \"a\" is forbidden"));
    let targets = vec![
        pod_ref("a"),
        pod_ref("ghost"),
        pod_ref("b"),
        pod_ref("c"),
        pod_ref("d"),
    ];
    let out = t
        .run(delete_of(targets), Initiator::Ui)
        .expect("the batch itself ran");
    let report = report(&out);
    let status: Vec<_> = report.rows.iter().map(|r| r.status.label()).collect();
    assert_eq!(status, ["Failed", "Skipped", "Done", "Done", "Done"]);
    assert_eq!(report.rows[0].status.message(), "pods \"a\" is forbidden");
    assert_eq!(report.rows[1].status.message(), "already gone");
    assert_eq!(
        out.message.as_deref(),
        Some("Deleted 3 of 5 objects, 1 skipped, 1 failed")
    );
    assert_eq!(report.failed_targets(), vec![pod_ref("a")]);
    let left: Vec<_> = t
        .port()
        .objects()
        .iter()
        .map(|o| o.name().to_owned())
        .collect();
    assert_eq!(left, ["a"], "only the refused one is left");
}

#[test]
fn a_delete_that_leaves_the_object_terminating_says_so() {
    let t = with_pods(&["a"]);
    let live = t.object("Pod", "a").unwrap();
    t.port().script().delete.push_ok(DeleteOutcome::Deleted);
    t.port()
        .script()
        .delete
        .push_ok(DeleteOutcome::Deleting(live));
    let out = t.run(delete_of(vec![pod_ref("a")]), Initiator::Ui).unwrap();
    assert_eq!(report(&out).rows[0].status.message(), "Deleting");
}

#[test]
fn each_object_has_an_audit_record_and_they_share_one_batch_id() {
    let t = with_pods(&["a", "b"]);
    t.port().insert(deployment("web"));
    let out = t
        .run(
            delete_of(vec![pod_ref("a"), pod_ref("b"), deployment_ref("web")]),
            Initiator::Agent,
        )
        .expect("delete");
    let batch = report(&out).batch;
    assert!(batch.starts_with("selection-"), "{batch}");

    let audit = t.h.audit();
    let per_object: Vec<_> = audit
        .iter()
        .filter(|r| matches!(r.cmd.as_ref(), "pod::Delete" | "resource::Delete"))
        .collect();
    assert_eq!(per_object.len(), 3, "{audit:?}");
    let mut seen: Vec<_> = per_object
        .iter()
        .map(|r| (r.cmd.to_string(), r.target.name.to_string()))
        .collect();
    seen.sort();
    assert_eq!(
        seen,
        [
            ("pod::Delete".to_owned(), "a".to_owned()),
            ("pod::Delete".to_owned(), "b".to_owned()),
            ("resource::Delete".to_owned(), "web".to_owned()),
        ]
    );
    for record in per_object {
        assert_eq!(record.initiator, Initiator::Agent);
        assert_eq!(record.outcome, AuditOutcome::Succeeded);
        assert!(!record.dry_run);
        assert_eq!(
            record.detail.as_deref(),
            Some(format!("batch={batch} via=selection::Delete").as_str())
        );
    }
    // The command is one more record: the cluster, the counts and the outcome.
    let umbrella = audit
        .iter()
        .find(|r| r.cmd.as_ref() == "selection::Delete")
        .unwrap();
    assert_eq!(umbrella.initiator, Initiator::Agent);
    assert_eq!(umbrella.outcome, AuditOutcome::Succeeded);
    let detail = umbrella.detail.as_deref().unwrap();
    assert!(
        detail.starts_with("items=3 kinds=Deployment:1,Pod:2"),
        "{detail}"
    );
    assert!(
        detail.ends_with(&format!("batch={batch} ok=3 skipped=0 failed=0")),
        "{detail}"
    );
    // Every record of the batch, the command's own included, carries the id as a field too.
    assert!(
        audit
            .iter()
            .all(|r| r.batch_id.as_deref() == Some(batch.as_str())),
        "{audit:?}"
    );
}

#[test]
fn a_failed_row_is_audited_as_failed() {
    let t = with_pods(&["a", "b"]);
    t.port().script().delete.push_err(OxiError::forbidden("no"));
    t.run(delete_of(vec![pod_ref("a"), pod_ref("b")]), Initiator::Ui)
        .unwrap();
    let audit = t.h.audit();
    let outcome_of = |name: &str| {
        audit
            .iter()
            .find(|r| r.cmd.as_ref() == "pod::Delete" && &*r.target.name == name)
            .map(|r| r.outcome)
    };
    assert_eq!(outcome_of("a"), Some(AuditOutcome::Failed));
    assert_eq!(outcome_of("b"), Some(AuditOutcome::Succeeded));
}

#[test]
fn a_dry_run_rehearses_every_object_and_deletes_none() {
    let t = with_pods(&["a", "b"]);
    let ctx = DispatchContext::new(Initiator::Agent, "agent").with_dry_run(true);
    let out =
        t.h.confirm_and_run(delete_of(vec![pod_ref("a"), pod_ref("b")]), ctx)
            .unwrap()
            .completed()
            .unwrap();
    let report = report(&out);
    assert!(report.dry_run);
    assert_eq!(
        out.message.as_deref(),
        Some("Dry run: 2 objects can be deleted")
    );
    assert_eq!(
        t.writes(),
        [("delete", "a".into(), true), ("delete", "b".into(), true)]
    );
    assert_eq!(t.port().objects().len(), 2);
    assert!(t.h.audit().iter().all(|r| r.dry_run));
}

#[test]
fn a_repeated_object_is_deleted_once() {
    let t = with_pods(&["a"]);
    let out = t
        .run(delete_of(vec![pod_ref("a"), pod_ref("a")]), Initiator::Ui)
        .unwrap();
    assert_eq!(report(&out).rows.len(), 1);
    // A rehearsal and a delete, once.
    assert_eq!(t.writes().len(), 2);
}

#[test]
fn an_empty_selection_and_one_of_two_clusters_are_refused_before_anything_is_sent() {
    let t = with_pods(&["a"]);
    t.h.connect("b", false);
    let empty = t.h.dispatch(
        delete_of(Vec::new()),
        DispatchContext::new(Initiator::Ui, "alice"),
    );
    assert!(
        matches!(empty, Err(DispatchError::NoCluster(_))),
        "{empty:?}"
    );

    let other = ResourceRef::namespaced(id("b"), Gvk::new("", "v1", "Pod"), "default", "x");
    let mixed = t.run(delete_of(vec![pod_ref("a"), other]), Initiator::Ui);
    let error = OxiError::from(mixed.unwrap_err());
    assert_eq!(error.kind(), oxikube_domain::ErrorKind::Validation);
    assert!(
        error.message().contains("more than one cluster"),
        "{}",
        error.message()
    );
    assert!(t.writes().is_empty());
    assert!(t.h.writes("b").is_empty());
}

#[test]
fn a_selection_past_the_limit_is_refused_before_anything_is_sent() {
    let t = Selected::new(false);
    let too_many: Vec<_> = (0..=MAX_BATCH).map(|i| pod_ref(&format!("p{i}"))).collect();
    let error = OxiError::from(t.run(delete_of(too_many), Initiator::Ui).unwrap_err());
    assert_eq!(error.kind(), oxikube_domain::ErrorKind::Validation);
    assert!(error.message().contains("at most"), "{}", error.message());
    assert!(t.writes().is_empty());
}

#[test]
fn thousands_of_objects_get_a_row_each_in_the_order_asked() {
    let t = Selected::new(false);
    let names: Vec<String> = (0..1_500).map(|i| format!("p{i:04}")).collect();
    for name in &names {
        t.port().insert(pod(name, "default", json!({})));
    }
    let targets: Vec<_> = names.iter().map(|n| pod_ref(n)).collect();
    let out = t.run(delete_of(targets.clone()), Initiator::Ui).unwrap();
    let report = report(&out);
    assert_eq!(report.rows.len(), 1_500);
    assert_eq!(report.succeeded(), 1_500);
    assert!(
        report
            .rows
            .iter()
            .zip(&targets)
            .all(|(row, target)| &row.target == target)
    );
    assert!(t.port().objects().is_empty());
}
