//! `selection::Label`: the merge patch of labels or annotations for every object, validated once,
//! audited without values.

use oxikube_domain::ErrorKind;
use oxikube_domain::OxiError;
use oxikube_domain::audit::Initiator;
use oxikube_domain::selection::{LabelEdit, LabelField};
use oxikube_ports::PatchKind;

use super::*;
use crate::selection::BulkReport;

fn label(targets: Vec<ResourceRef>, field: LabelField, edits: Vec<LabelEdit>) -> Command {
    Command::SelectionLabel {
        targets,
        field,
        edits,
    }
}

fn labels_of(t: &Selected, name: &str) -> Value {
    t.object("Pod", name).unwrap().to_value()["metadata"]["labels"].clone()
}

#[test]
fn labels_are_added_changed_and_removed_on_every_object() {
    let t = Selected::new(false);
    t.port().insert(pod(
        "a",
        "default",
        json!({"app": "web", "tier": "old", "gone": "x"}),
    ));
    t.port()
        .insert(pod("b", "default", json!({"app": "api", "gone": "y"})));
    let edits = vec![
        LabelEdit::set("tier", "backend"),
        LabelEdit::set("team", "data"),
        LabelEdit::remove("gone"),
    ];
    let out = t
        .run(
            label(vec![pod_ref("a"), pod_ref("b")], LabelField::Labels, edits),
            Initiator::Ui,
        )
        .expect("label");
    assert_eq!(out.message.as_deref(), Some("Labelled 2 objects"));
    assert_eq!(
        labels_of(&t, "a"),
        json!({"app": "web", "tier": "backend", "team": "data"})
    );
    assert_eq!(
        labels_of(&t, "b"),
        json!({"app": "api", "tier": "backend", "team": "data"})
    );
}

#[test]
fn the_request_is_a_merge_patch_with_null_for_a_removal() {
    let t = Selected::new(false);
    t.port().insert(pod("a", "default", json!({"gone": "x"})));
    t.run(
        label(
            vec![pod_ref("a")],
            LabelField::Labels,
            vec![LabelEdit::set("team", "data"), LabelEdit::remove("gone")],
        ),
        Initiator::Ui,
    )
    .unwrap();
    let patches: Vec<_> = t
        .port()
        .mutating_calls()
        .into_iter()
        .map(|call| match call {
            ResourceCall::Patch { patch, options, .. } => (options.dry_run, patch.kind, patch.body),
            other => panic!("{other:?}"),
        })
        .collect();
    let body = json!({"metadata": {"labels": {"team": "data", "gone": null}}});
    assert_eq!(
        patches,
        [
            (true, PatchKind::Merge, body.clone()),
            (false, PatchKind::Merge, body)
        ]
    );
}

#[test]
fn annotations_use_the_same_command_with_the_other_field() {
    let t = Selected::new(false);
    t.port().insert(pod("a", "default", json!({"app": "web"})));
    t.run(
        label(
            vec![pod_ref("a")],
            LabelField::Annotations,
            vec![LabelEdit::set(
                "example.io/note",
                "free text: with {punctuation} and spaces",
            )],
        ),
        Initiator::Ui,
    )
    .unwrap();
    let live = t.object("Pod", "a").unwrap().to_value();
    assert_eq!(
        live["metadata"]["annotations"]["example.io/note"],
        "free text: with {punctuation} and spaces"
    );
    assert_eq!(
        live["metadata"]["labels"],
        json!({"app": "web"}),
        "labels are untouched"
    );
}

#[test]
fn any_kind_can_be_labelled_including_cluster_scoped_ones() {
    let t = Selected::new(false);
    t.port().insert(namespace("dev"));
    t.port().insert(deployment("web"));
    let out = t
        .run(
            label(
                vec![namespace_ref("dev"), deployment_ref("web")],
                LabelField::Labels,
                vec![LabelEdit::set("env", "dev")],
            ),
            Initiator::Ui,
        )
        .unwrap();
    assert!(BulkReport::from_data(out.data.as_ref()).unwrap().failed() == 0);
    assert_eq!(
        t.object("Namespace", "dev").unwrap().to_value()["metadata"]["labels"]["env"],
        "dev"
    );
}

#[test]
fn an_invalid_key_or_value_fails_once_before_any_request() {
    let t = Selected::new(false);
    t.port().insert(pod("a", "default", json!({})));
    for edits in [
        vec![LabelEdit::set("bad key", "v")],
        vec![LabelEdit::set("ok", "has space")],
        vec![LabelEdit::set("a", "1"), LabelEdit::remove("a")],
        Vec::new(),
    ] {
        let error = OxiError::from(
            t.run(
                label(vec![pod_ref("a")], LabelField::Labels, edits),
                Initiator::Ui,
            )
            .unwrap_err(),
        );
        assert_eq!(error.kind(), ErrorKind::Validation, "{}", error.message());
    }
    assert!(t.writes().is_empty());
}

#[test]
fn the_audit_records_name_the_field_and_the_counts_never_the_values() {
    let t = Selected::new(false);
    t.port().insert(pod("a", "default", json!({})));
    t.run(
        label(
            vec![pod_ref("a")],
            LabelField::Annotations,
            vec![
                LabelEdit::set("example.io/token", "hunter2-not-a-secret"),
                LabelEdit::remove("old"),
            ],
        ),
        Initiator::Ui,
    )
    .unwrap();
    let audit = t.h.audit();
    let row = audit
        .iter()
        .find(|r| r.cmd.as_ref() == "selection::Label" && &*r.target.name == "a")
        .expect("the object's record");
    let detail = row.detail.as_deref().unwrap();
    assert!(
        detail.contains("field=annotations set=1 remove=1"),
        "{detail}"
    );
    for record in &audit {
        let text = format!("{record:?}");
        assert!(!text.contains("hunter2"), "{text}");
        assert!(!text.contains("example.io/token"), "{text}");
    }
}

#[test]
fn an_object_that_vanished_is_skipped() {
    let t = Selected::new(false);
    t.port().insert(pod("a", "default", json!({})));
    let out = t
        .run(
            label(
                vec![pod_ref("ghost"), pod_ref("a")],
                LabelField::Labels,
                vec![LabelEdit::set("k", "v")],
            ),
            Initiator::Ui,
        )
        .unwrap();
    let report = BulkReport::from_data(out.data.as_ref()).unwrap();
    assert_eq!(report.rows[0].status.label(), "Skipped");
    assert_eq!(report.rows[1].status.label(), "Done");
    assert_eq!(labels_of(&t, "a"), json!({"k": "v"}));
}
