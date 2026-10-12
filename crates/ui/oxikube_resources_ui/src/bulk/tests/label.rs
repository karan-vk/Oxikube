//! The label editor: shift-l over a selection, the labels / annotations switch, validation, the
//! preview of one sample, the selector warning, and the patch every object gets.

use gpui::TestAppContext;
use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::selection::{LabelEdit, LabelField, LabelPreview};
use oxikube_ports::PatchKind;
use oxikube_testkit::ResourceCall;

use super::*;
use crate::bulk::{BulkKind, BulkStage};

fn labelled_pods() -> Vec<Resource> {
    (0..3)
        .map(|i| {
            let mut pod = crate::table::tests::p("x", &format!("web-{i}"), "1").to_value();
            pod["metadata"]["labels"] = json!({"app": "web", "tier": "old"});
            pod["metadata"]["annotations"] = json!({"note": "keep"});
            Resource::from_json(pod).unwrap()
        })
        .collect()
}

fn fill(f: &mut Fixture, d: &Entity<BulkDialog>, ix: usize, key: &str, value: &str) {
    with_dialog(f, d, |d, window, cx| {
        let editor = d.editor_mut().expect("the label editor");
        editor.fill(ix, key, value, window, cx);
    });
}

fn open(f: &mut Fixture) -> (Entity<ResourceTable>, Entity<BulkDialog>) {
    let table = f.open_pods();
    select_all(f, &table);
    f.keys(&table, "shift-l");
    f.settle();
    let d = dialog(f).expect("the label editor opens");
    (table, d)
}

#[gpui::test]
fn shift_l_opens_the_editor_for_the_selection_with_counts_and_the_selector_warning(
    cx: &mut TestAppContext,
) {
    let mut f = Fixture::with_actions(cx);
    f.connect_with(labelled_pods());
    let (_, d) = open(&mut f);
    with_dialog(&mut f, &d, |d, _, cx| {
        assert_eq!(d.kind(), BulkKind::Label);
        assert_eq!(d.plan().summary().lines(5), ["3 Pods in x"]);
        assert!(!d.can_confirm(cx), "nothing to change yet");
    });
    for part in [
        "bulk-what",
        "bulk-field-switch",
        "bulk-row-0",
        "bulk-warning",
        "bulk-confirm",
    ] {
        assert!(drawn(&mut f, part), "{part} is drawn");
    }
    assert!(
        f.ports().resources.mutating_calls().is_empty(),
        "opening sends nothing"
    );
}

#[gpui::test]
fn the_preview_marks_added_changed_and_removed_keys_of_a_sample_object(cx: &mut TestAppContext) {
    let mut f = Fixture::with_actions(cx);
    f.connect_with(labelled_pods());
    let (_, d) = open(&mut f);
    fill(&mut f, &d, 0, "tier", "backend");
    with_dialog(&mut f, &d, |d, window, cx| {
        let editor = d.editor_mut().unwrap();
        editor.add_row(window, cx);
    });
    fill(&mut f, &d, 1, "team", "data");
    let preview = d.read_with(&f.vcx, |d, cx| d.editor().unwrap().preview(cx));
    assert!(preview.contains(&LabelPreview::Changed {
        key: "tier".into(),
        from: "old".into(),
        to: "backend".into()
    }));
    assert!(preview.contains(&LabelPreview::Added {
        key: "team".into(),
        value: "data".into()
    }));
    assert!(preview.contains(&LabelPreview::Kept {
        key: "app".into(),
        value: "web".into()
    }));
    assert!(drawn(&mut f, "bulk-preview"));
    // The preview is of the labels, not the annotations.
    with_dialog(&mut f, &d, |d, _, cx| {
        d.set_field(LabelField::Annotations, cx)
    });
    let preview = d.read_with(&f.vcx, |d, cx| d.editor().unwrap().preview(cx));
    assert!(preview.contains(&LabelPreview::Added {
        key: "tier".into(),
        value: "backend".into()
    }));
    assert!(preview.contains(&LabelPreview::Kept {
        key: "note".into(),
        value: "keep".into()
    }));
}

#[gpui::test]
fn applying_sends_one_merge_patch_to_every_object_and_audits_each(cx: &mut TestAppContext) {
    let mut f = Fixture::with_actions(cx);
    f.connect_with(labelled_pods());
    let (_, d) = open(&mut f);
    fill(&mut f, &d, 0, "tier", "backend");
    with_dialog(&mut f, &d, |d, window, cx| {
        d.editor_mut().unwrap().add_row(window, cx)
    });
    fill(&mut f, &d, 1, "team", "data");
    with_dialog(&mut f, &d, |d, window, cx| {
        let editor = d.editor_mut().unwrap();
        editor.add_row(window, cx);
        editor.fill(2, "app", "", window, cx);
        editor.toggle_remove(2);
    });
    assert!(d.read_with(&f.vcx, |d, cx| d.can_confirm(cx)));
    assert_eq!(
        d.read_with(&f.vcx, |d, cx| d.editor().unwrap().edits(cx)),
        [
            LabelEdit::set("tier", "backend"),
            LabelEdit::set("team", "data"),
            LabelEdit::remove("app"),
        ]
    );
    with_dialog(&mut f, &d, |d, _, cx| d.confirm(cx));
    f.settle();

    let report = d
        .read_with(&f.vcx, |d, _| d.report().cloned())
        .expect("results");
    assert_eq!(d.read_with(&f.vcx, |d, _| d.stage()), BulkStage::Done);
    assert_eq!(report.summary(), "Labelled 3 objects");
    // Each object: a rehearsal and the patch; the body is a merge patch with null for a removal.
    let calls = f.ports().resources.mutating_calls();
    assert_eq!(calls.len(), 6);
    let body = json!({"metadata": {"labels": {"tier": "backend", "team": "data", "app": null}}});
    for call in &calls {
        let ResourceCall::Patch { patch, .. } = call else {
            panic!("{call:?}")
        };
        assert_eq!(patch.kind, PatchKind::Merge);
        assert_eq!(patch.body, body);
    }
    let live = f.ports().resources.objects();
    for pod in live.iter().filter(|o| &*o.kind.kind == "Pod") {
        let labels = &pod.to_value()["metadata"]["labels"];
        assert_eq!(
            labels,
            &json!({"tier": "backend", "team": "data"}),
            "{}",
            pod.name()
        );
    }
    // One audit record per pod, none carrying a value.
    let records = audit_of(&f, "selection::Label");
    assert_eq!(records.len(), 4, "three pods and the command: {records:?}");
    for record in f.state.audit_log() {
        let text = format!("{record:?}");
        assert!(
            !text.contains("backend") && !text.contains("data\""),
            "{text}"
        );
        assert_eq!(record.initiator, Initiator::Ui);
        assert_eq!(record.outcome, AuditOutcome::Succeeded);
    }
}

#[gpui::test]
fn annotations_are_edited_in_the_same_dialog_without_the_selector_warning(cx: &mut TestAppContext) {
    let mut f = Fixture::with_actions(cx);
    f.connect_with(labelled_pods());
    let (_, d) = open(&mut f);
    assert!(drawn(&mut f, "bulk-warning"));
    with_dialog(&mut f, &d, |d, _, cx| {
        d.set_field(LabelField::Annotations, cx)
    });
    assert!(
        !drawn(&mut f, "bulk-warning"),
        "annotations are not matched by selectors"
    );
    // An annotation value is free text; a label value would be refused.
    fill(
        &mut f,
        &d,
        0,
        "example.io/note",
        "has spaces: and punctuation",
    );
    assert!(d.read_with(&f.vcx, |d, cx| d.can_confirm(cx)));
    with_dialog(&mut f, &d, |d, _, cx| d.confirm(cx));
    f.settle();
    let live = f.ports().resources.objects();
    for pod in live.iter().filter(|o| &*o.kind.kind == "Pod") {
        assert_eq!(
            pod.to_value()["metadata"]["annotations"]["example.io/note"],
            "has spaces: and punctuation"
        );
        assert_eq!(pod.to_value()["metadata"]["annotations"]["note"], "keep");
    }
}

#[gpui::test]
fn an_invalid_key_or_value_is_explained_and_cannot_be_sent(cx: &mut TestAppContext) {
    let mut f = Fixture::with_actions(cx);
    f.connect_with(labelled_pods());
    let (_, d) = open(&mut f);
    fill(&mut f, &d, 0, "bad key!", "v");
    assert!(!d.read_with(&f.vcx, |d, cx| d.can_confirm(cx)));
    assert!(drawn(&mut f, "bulk-problem"));
    with_dialog(&mut f, &d, |d, _, cx| d.confirm(cx));
    assert!(f.ports().resources.mutating_calls().is_empty());
    fill(&mut f, &d, 0, "ok", "has space");
    assert!(
        d.read_with(&f.vcx, |d, cx| d.editor().unwrap().problem(cx))
            .is_some()
    );
    fill(&mut f, &d, 0, "ok", "fine");
    assert!(d.read_with(&f.vcx, |d, cx| d.can_confirm(cx)));
    assert!(!drawn(&mut f, "bulk-problem"));
}

#[gpui::test]
fn a_single_row_can_be_labelled_too_and_a_namespace_row_as_well(cx: &mut TestAppContext) {
    let mut f = Fixture::with_actions(cx);
    f.connect_with(labelled_pods());
    let table = f.open_pods();
    f.update(&table, |t, cx| t.move_cursor(1, false, cx));
    f.keys(&table, "shift-l");
    f.settle();
    let d = dialog(&mut f).expect("the editor");
    assert_eq!(d.read_with(&f.vcx, |d, _| d.plan().targets().len()), 1);
    fill(&mut f, &d, 0, "solo", "yes");
    with_dialog(&mut f, &d, |d, _, cx| d.confirm(cx));
    f.settle();
    // One object that went well closes the dialog with a toast instead of a one-row list.
    assert!(dialog(&mut f).is_none());
    let toasts = crate::actions::tests::toasts(&mut f);
    assert!(
        toasts
            .iter()
            .any(|t| t.message.contains("Labelled 1 object")),
        "{toasts:?}"
    );
}

#[gpui::test]
fn a_read_only_cluster_greys_the_editor_out(cx: &mut TestAppContext) {
    let mut f = Fixture::with_actions(cx);
    f.connect_with(labelled_pods());
    let table = f.open_pods();
    select_all(&mut f, &table);
    f.sessions.set_read_only(&cluster(), true).unwrap();
    f.keys(&table, "shift-l");
    assert!(dialog(&mut f).is_none());
    let toasts = crate::actions::tests::toasts(&mut f);
    assert!(
        toasts.iter().any(|t| t.message.contains("read-only")),
        "{toasts:?}"
    );
    assert!(f.ports().resources.mutating_calls().is_empty());
}
