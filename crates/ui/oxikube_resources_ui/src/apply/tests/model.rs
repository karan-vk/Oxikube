//! The model alone: rows from a plan, results folded in, the filter, the expansion, the cursor,
//! the counts and the copied text.

use std::time::Duration;

use oxikube_app::apply::{ObjectResult, ObjectStatus};
use oxikube_domain::apply::{ApplyPlan, PlanItem};
use oxikube_domain::manifests::{ManifestDoc, SourceId};
use serde_json::json;

use crate::apply::model::{Activity, Line, Model, Stage};

fn item(kind: &str, name: &str, namespace: Option<&str>, line: usize) -> PlanItem {
    let mut json = json!({"apiVersion": "v1", "kind": kind, "metadata": {"name": name}});
    if let Some(namespace) = namespace {
        json["metadata"]["namespace"] = json!(namespace);
    }
    PlanItem {
        doc: ManifestDoc {
            source: SourceId::new("/m/a.yaml"),
            line,
            json,
        },
        kind_rank: 0,
        depends_on: Vec::new(),
    }
}

fn plan() -> ApplyPlan {
    ApplyPlan {
        items: vec![
            item("Namespace", "shop", None, 1),
            item("ConfigMap", "a", Some("shop"), 5),
            item("ConfigMap", "b", Some("shop"), 12),
        ],
    }
}

fn result(index: usize, dry_run: bool, status: ObjectStatus) -> ObjectResult {
    ObjectResult {
        index,
        label: format!("object {index}"),
        target: None,
        dry_run,
        status,
        method: None,
    }
}

fn failed(reason: &str, fields: &[&str]) -> ObjectStatus {
    ObjectStatus::Failed {
        reason: reason.to_owned(),
        fields: fields.iter().map(|f| (*f).to_owned()).collect(),
        conflicts: Vec::new(),
    }
}

#[test]
fn a_plan_makes_one_row_per_object_with_its_namespace_and_line() {
    let mut model = Model::new();
    model.planned(&plan());
    assert_eq!(model.stage(), Stage::Review);
    let rows = model.rows();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].namespace, None, "a Namespace is cluster-scoped");
    assert_eq!(rows[1].namespace.as_deref(), Some("shop"));
    assert_eq!(
        (rows[1].kind.as_str(), rows[1].name.as_str(), rows[1].line),
        ("ConfigMap", "a", 5)
    );
    assert_eq!(rows[1].label(), "ConfigMap shop/a");
    assert_eq!(rows[0].label(), "Namespace shop");
}

#[test]
fn results_fold_in_by_index_and_the_apply_answer_wins_over_the_dry_run() {
    let mut model = Model::new();
    model.planned(&plan());
    model.begin_dry_run();
    model.record(
        result(1, true, ObjectStatus::Created),
        Duration::from_millis(40),
    );
    assert_eq!(model.rows()[1].shown(), Some(&ObjectStatus::Created));
    assert_eq!(model.summary_line(), "1 created");
    model.begin_apply();
    assert_eq!(model.stage(), Stage::Result);
    assert_eq!(model.activity(), Activity::Applying);
    assert!(
        model.rows()[1].shown().is_some(),
        "the dry run still shows until the apply answers"
    );
    model.record(
        result(1, false, ObjectStatus::Configured),
        Duration::from_millis(900),
    );
    assert_eq!(model.rows()[1].shown(), Some(&ObjectStatus::Configured));
    assert_eq!(model.rows()[1].elapsed, Some(Duration::from_millis(900)));
    model.record(result(99, false, ObjectStatus::Created), Duration::ZERO);
    assert_eq!(model.rows().len(), 3, "a result for no object is dropped");
}

#[test]
fn errors_only_keeps_failed_and_skipped_rows() {
    let mut model = Model::new();
    model.planned(&plan());
    model.begin_apply();
    model.record(result(0, false, ObjectStatus::Created), Duration::ZERO);
    model.record(result(1, false, failed("no", &[])), Duration::ZERO);
    model.record(
        result(
            2,
            false,
            ObjectStatus::Skipped {
                reason: "needs a".into(),
            },
        ),
        Duration::ZERO,
    );
    assert_eq!(model.lines().len(), 3);
    model.set_errors_only(true);
    assert_eq!(model.lines(), [Line::Row(1), Line::Row(2)]);
    assert_eq!(model.failures(), 2);
    assert_eq!(
        model.failed_keys(),
        [
            ("ConfigMap".to_owned(), "a".to_owned()),
            ("ConfigMap".to_owned(), "b".to_owned())
        ]
    );
}

#[test]
fn an_expanded_failure_adds_its_message_and_field_paths_as_lines() {
    let mut model = Model::new();
    model.planned(&plan());
    model.begin_apply();
    model.record(
        result(
            1,
            false,
            failed(
                "ConfigMap \"a\" is invalid: spec.x: Required value",
                &["spec.x", "metadata.name"],
            ),
        ),
        Duration::ZERO,
    );
    model.record(result(0, false, ObjectStatus::Created), Duration::ZERO);
    model.toggle_expanded(0);
    assert!(
        !model.is_expanded(0),
        "a row that did not fail has nothing to open"
    );
    model.toggle_expanded(1);
    assert!(model.is_expanded(1));
    let lines = model.lines().to_vec();
    assert_eq!(lines[0], Line::Row(0));
    assert_eq!(lines[1], Line::Row(1));
    assert!(matches!(&lines[2], Line::Detail(1, text) if text.contains("Required value")));
    assert_eq!(lines[3], Line::Detail(1, "field: spec.x".into()));
    assert_eq!(lines[4], Line::Detail(1, "field: metadata.name".into()));
    model.toggle_expanded(1);
    assert_eq!(model.lines().len(), 3);
}

#[test]
fn the_cursor_steps_through_the_shown_rows() {
    let mut model = Model::new();
    model.planned(&plan());
    model.step(1);
    assert_eq!(model.selected(), Some(0));
    model.step(1);
    model.step(1);
    model.step(1);
    assert_eq!(model.selected(), Some(2), "it stops at the last");
    model.step(isize::MIN);
    assert_eq!(model.selected(), Some(0));
    model.step(isize::MAX);
    assert_eq!(model.selected(), Some(2));
}

#[test]
fn copied_text_names_objects_statuses_and_messages_only() {
    let mut model = Model::new();
    model.planned(&plan());
    model.begin_apply();
    model.record(
        result(0, false, ObjectStatus::Created),
        Duration::from_millis(1500),
    );
    model.record(
        result(1, false, failed("it is invalid", &["data.k"])),
        Duration::from_millis(200),
    );
    let text = model.as_text();
    assert!(text.contains("created\tNamespace shop\t1.5s\n"), "{text}");
    assert!(
        text.contains("failed\tConfigMap shop/a\t0.2s\n  it is invalid\n  field: data.k\n"),
        "{text}"
    );
    assert!(text.contains("waiting\tConfigMap shop/b\n"), "{text}");
}

#[test]
fn a_run_that_ends_early_marks_what_has_no_answer_skipped() {
    let mut model = Model::new();
    model.planned(&plan());
    model.begin_dry_run();
    model.record(result(0, true, ObjectStatus::Created), Duration::ZERO);
    model.skip_unanswered("cancelled");
    assert_eq!(model.count("skipped"), 2);
    assert_eq!(model.count("created"), 1);
}
