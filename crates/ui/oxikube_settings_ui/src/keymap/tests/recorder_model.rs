//! The recorder's state as data: recording, the payload it saves, and when Save is allowed.

use oxikube_domain::command::KeymapSource;
use oxikube_keymap::edit::{ContextModel, EditPreview};
use oxikube_keymap::{ContextStacks, KeymapLayer, KeymapRow, KnownNames, RowStatus};

use crate::keymap::recorder::{ConflictChoice, MAX_KEYSTROKES, RecorderModel, Verdict};

fn row(action: &str, context: Option<&str>, keys: &str, layer: Option<KeymapLayer>) -> KeymapRow {
    KeymapRow {
        action: action.to_owned(),
        title: action.to_owned(),
        context: context.map(str::to_owned),
        keystrokes: keys.split_whitespace().map(str::to_owned).collect(),
        data: None,
        source: layer,
        status: if layer.is_some() {
            RowStatus::Active
        } else {
            RowStatus::Unbound
        },
    }
}

fn writable() -> Result<EditPreview, String> {
    Ok(EditPreview {
        text: String::new(),
        diagnostics: Vec::new(),
        introduced: Vec::new(),
    })
}

#[test]
fn an_unbound_command_gets_a_binding_added_in_the_context_typed() {
    let mut m = RecorderModel::for_row(&row("a::Run", None, "", None));
    assert!(m.is_recording());
    assert!(m.edit().is_none(), "nothing recorded yet");
    assert!(m.push("ctrl-k".into()));
    assert!(m.push("ctrl-s".into()));
    m.set_context(" ResourceTable ");
    let edit = m.edit().unwrap();
    assert_eq!(edit.keystrokes, "ctrl-k ctrl-s");
    assert_eq!(edit.context.as_deref(), Some("ResourceTable"));
    assert!(edit.replaces.is_none());
    assert!(!m.can_save(), "not validated yet");
    m.set_preview(writable());
    assert!(m.can_save());
}

#[test]
fn a_bound_row_is_replaced_and_recording_it_again_is_no_change() {
    let mut m = RecorderModel::for_row(&row(
        "a::Run",
        Some("Workspace"),
        "cmd-r",
        Some(KeymapLayer::Default),
    ));
    assert_eq!(m.context(), "Workspace");
    m.push("cmd-r".into());
    m.set_preview(writable());
    assert!(m.is_unchanged() && !m.can_save());
    m.restart();
    m.push("cmd-shift-r".into());
    m.set_preview(writable());
    let edit = m.edit().unwrap();
    let replaces = edit.replaces.unwrap();
    assert_eq!(replaces.source, KeymapSource::Default);
    assert_eq!(replaces.keystrokes, "cmd-r");
    assert!(m.can_save());
}

#[test]
fn recording_stops_at_the_longest_sequence_and_clear_forgets() {
    let mut m = RecorderModel::for_row(&row("a::Run", None, "", None));
    for i in 0..MAX_KEYSTROKES {
        let going_on = m.push(format!("f{}", i + 1));
        assert_eq!(going_on, i + 1 < MAX_KEYSTROKES);
    }
    assert!(!m.is_recording());
    assert!(!m.push("f9".into()), "not recording any more");
    assert_eq!(m.keystrokes().len(), MAX_KEYSTROKES);
    m.clear();
    assert!(m.keystrokes().is_empty() && !m.is_recording());
}

#[test]
fn replace_unbinds_the_conflicts_and_keep_both_leaves_them() {
    let rows = [
        row(
            "a::Other",
            Some("Workspace"),
            "ctrl-k",
            Some(KeymapLayer::Default),
        ),
        row(
            "a::Mine",
            Some("ResourceTable"),
            "ctrl-j",
            Some(KeymapLayer::User),
        ),
    ];
    let contexts = ContextModel::of(
        ContextStacks::of(&[&["Workspace", "Pane", "ResourceTable"]]),
        KnownNames::of(["Workspace", "Pane", "ResourceTable"], ["Editing"]),
    );
    let mut m = RecorderModel::for_row(&rows[1]);
    m.push("ctrl-k".into());
    m.refresh_conflicts(&contexts, &rows);
    assert_eq!(m.conflicts().len(), 1);
    assert_eq!(m.conflicts()[0].row.action, "a::Other");
    assert!(m.edit().unwrap().unbind.is_empty());
    m.set_choice(ConflictChoice::Replace);
    assert_eq!(
        *m.verdict(),
        Verdict::Pending,
        "a new answer is validated again"
    );
    let unbind = m.edit().unwrap().unbind;
    assert_eq!(unbind.len(), 1);
    assert_eq!(unbind[0].context.as_deref(), Some("Workspace"));
    assert_eq!(unbind[0].source, KeymapSource::Default);
}

#[test]
fn an_overwritten_user_binding_is_reported_and_leaves_no_choice() {
    let rows = [
        row(
            "a::Mine",
            Some("ResourceTable"),
            "ctrl-k",
            Some(KeymapLayer::User),
        ),
        row(
            "a::Other",
            Some("Workspace"),
            "ctrl-k",
            Some(KeymapLayer::Default),
        ),
    ];
    let contexts = ContextModel::of(
        ContextStacks::of(&[&["Workspace", "Pane", "ResourceTable"]]),
        KnownNames::of(["Workspace", "Pane", "ResourceTable"], ["Editing"]),
    );
    let mut m = RecorderModel::for_row(&row("a::Run", None, "", None));
    m.push("ctrl-k".into());
    m.set_context("ResourceTable");
    m.refresh_conflicts(&contexts, &rows[..1]);
    let overwritten: Vec<_> = m.overwritten().map(|c| c.row.action.clone()).collect();
    assert_eq!(overwritten, ["a::Mine"]);
    assert!(
        !m.has_choice(),
        "Keep both would write the same file as Replace"
    );
    // A conflict that can be kept brings the choice back; the overwritten one stays reported.
    m.refresh_conflicts(&contexts, &rows);
    assert!(m.has_choice());
    assert_eq!(m.overwritten().count(), 1);
}

#[test]
fn a_blocked_verdict_disables_save_and_carries_the_reasons() {
    let mut m = RecorderModel::for_row(&row("a::Run", None, "", None));
    m.push("ctrl-k".into());
    m.set_preview(Err("keymap.json cannot be edited".into()));
    assert!(!m.can_save());
    assert_eq!(
        *m.verdict(),
        Verdict::Blocked(vec!["keymap.json cannot be edited".into()])
    );
}

#[test]
fn a_row_out_of_force_gets_a_binding_added_never_replaced() {
    // Replacing an overridden row would find the binding in force at its keys (the user's
    // override) and overwrite it with `null`: the recorder adds a binding instead.
    for status in [RowStatus::Overridden, RowStatus::UnboundByUser] {
        for layer in [KeymapLayer::Default, KeymapLayer::User] {
            let mut overridden = row("x::Old", Some("Workspace"), "cmd-k", Some(layer));
            overridden.status = status;
            let mut m = RecorderModel::for_row(&overridden);
            assert!(m.editing().is_none(), "{status:?} {layer:?}");
            assert_eq!(m.context(), "Workspace", "starts in the row's context");
            m.push("cmd-j".into());
            let edit = m.edit().expect("keys recorded");
            assert!(edit.replaces.is_none(), "{status:?} {layer:?}: {edit:?}");
            assert!(!m.is_unchanged());
        }
    }
}
