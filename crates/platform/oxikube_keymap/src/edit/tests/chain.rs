//! Conflicts across the context chain: same context, parent and child, prefix shadowing, and the
//! non-conflicts (siblings, contradictory flags, other keys).

use oxikube_domain::command::{KeymapBindingEdit, KeymapBindingRef, KeymapSource};

use crate::edit::{
    ConflictKind, ContextModel, apply_operations, ref_of_row, set_binding_operations,
};
use crate::known::KnownNames;
use crate::layer::KeymapLayer;
use crate::rows::{KeymapRow, RowStatus};
use crate::stacks::ContextStacks;

fn model() -> ContextModel {
    ContextModel::of(
        ContextStacks::of(&[
            &["Workspace"],
            &[
                "Workspace",
                "Dock",
                "ClusterTab",
                "Workspace",
                "Pane",
                "ResourceTable",
            ],
            &[
                "Workspace",
                "Dock",
                "ClusterTab",
                "Workspace",
                "Pane",
                "Terminal",
            ],
            &["Workspace", "Catalog"],
        ]),
        KnownNames::of(
            [
                "Workspace",
                "Dock",
                "ClusterTab",
                "Pane",
                "ResourceTable",
                "Terminal",
                "Catalog",
            ],
            ["Editing", "kind"],
        ),
    )
}

fn row(action: &str, context: Option<&str>, keys: &str, layer: KeymapLayer) -> KeymapRow {
    KeymapRow {
        action: action.to_owned(),
        title: action.to_owned(),
        context: context.map(str::to_owned),
        keystrokes: keys.split_whitespace().map(str::to_owned).collect(),
        data: None,
        source: Some(layer),
        status: RowStatus::Active,
    }
}

fn kinds(found: &[crate::edit::ChainConflict]) -> Vec<(String, ConflictKind)> {
    found
        .iter()
        .map(|c| (c.row.action.clone(), c.kind))
        .collect()
}

#[test]
fn the_same_context_and_a_parent_or_child_context_conflict() {
    let m = model();
    let rows = [
        row(
            "a::Same",
            Some("ResourceTable"),
            "ctrl-k",
            KeymapLayer::Default,
        ),
        row(
            "a::Parent",
            Some("Workspace"),
            "ctrl-k",
            KeymapLayer::Default,
        ),
        row("a::Tab", Some("ClusterTab"), "ctrl-k", KeymapLayer::User),
        row("a::Everywhere", None, "ctrl-k", KeymapLayer::Default),
        row(
            "a::Sibling",
            Some("Terminal"),
            "ctrl-k",
            KeymapLayer::Default,
        ),
        row(
            "a::Outside",
            Some("Catalog"),
            "ctrl-k",
            KeymapLayer::Default,
        ),
        row(
            "a::OtherKeys",
            Some("ResourceTable"),
            "ctrl-l",
            KeymapLayer::Default,
        ),
    ];
    let found = m.conflicts("b::New", Some("ResourceTable"), "ctrl-k", &rows, None);
    assert_eq!(
        kinds(&found),
        [
            ("a::Same".to_owned(), ConflictKind::SameKeys),
            ("a::Parent".to_owned(), ConflictKind::SameKeys),
            ("a::Tab".to_owned(), ConflictKind::SameKeys),
            ("a::Everywhere".to_owned(), ConflictKind::SameKeys),
        ]
    );
    // From the parent's side, the children are in its chain.
    let found = m.conflicts("b::New", Some("Workspace"), "ctrl-k", &rows, None);
    let names: Vec<_> = kinds(&found).into_iter().map(|(n, _)| n).collect();
    for expected in [
        "a::Same",
        "a::Parent",
        "a::Sibling",
        "a::Outside",
        "a::Everywhere",
    ] {
        assert!(
            names.contains(&expected.to_owned()),
            "{expected}: {names:?}"
        );
    }
}

#[test]
fn a_prefix_shadows_the_longer_sequence_both_ways() {
    let m = model();
    let rows = [
        row(
            "a::GotoTop",
            Some("ResourceTable"),
            "g g",
            KeymapLayer::Default,
        ),
        row("a::Go", Some("ResourceTable"), "g", KeymapLayer::Default),
        row("a::Gx", Some("ResourceTable"), "g x", KeymapLayer::Default),
    ];
    let found = m.conflicts("b::New", Some("ResourceTable"), "g", &rows, None);
    assert_eq!(
        kinds(&found),
        [
            ("a::GotoTop".to_owned(), ConflictKind::ShadowsLonger),
            ("a::Go".to_owned(), ConflictKind::SameKeys),
            ("a::Gx".to_owned(), ConflictKind::ShadowsLonger),
        ]
    );
    let found = m.conflicts("b::New", Some("ResourceTable"), "g g", &rows, None);
    assert_eq!(
        kinds(&found),
        [
            ("a::GotoTop".to_owned(), ConflictKind::SameKeys),
            ("a::Go".to_owned(), ConflictKind::ShadowedByShorter),
        ]
    );
}

#[test]
fn siblings_contradictory_flags_and_values_never_conflict() {
    let m = model();
    assert!(!m.can_be_active_together(Some("ResourceTable"), Some("Terminal")));
    assert!(!m.can_be_active_together(
        Some("ResourceTable && Editing"),
        Some("ResourceTable && !Editing")
    ));
    assert!(!m.can_be_active_together(
        Some("ResourceTable && kind == Pod"),
        Some("ResourceTable && kind == Deployment")
    ));
    assert!(m.can_be_active_together(
        Some("ResourceTable && kind == Pod"),
        Some("ResourceTable && kind != Deployment")
    ));
    assert!(m.can_be_active_together(Some("ResourceTable && !Editing"), Some("Workspace")));
    assert!(m.can_be_active_together(None, Some("Terminal")));
    assert!(m.can_be_active_together(Some("Workspace > Terminal"), Some("Terminal")));
    assert!(!m.can_be_active_together(Some("Catalog > Terminal"), Some("Terminal")));
    assert!(
        !m.can_be_active_together(Some("((("), Some("Terminal")),
        "never loads"
    );
}

#[test]
fn an_unknown_context_is_placed_in_a_cluster_tab_pane() {
    let m = model();
    assert!(m.can_be_active_together(Some("MyView"), Some("ClusterTab")));
    assert!(!m.can_be_active_together(Some("MyView"), Some("Catalog")));
    assert!(!m.can_be_active_together(Some("MyView"), Some("ResourceTable")));
}

#[test]
fn the_binding_being_edited_the_same_action_and_rows_out_of_force_are_not_conflicts() {
    let m = model();
    let editing = row("b::New", Some("ResourceTable"), "ctrl-j", KeymapLayer::User);
    let mut overridden = row(
        "a::Old",
        Some("ResourceTable"),
        "ctrl-k",
        KeymapLayer::Default,
    );
    overridden.status = RowStatus::Overridden;
    let rows = [
        editing.clone(),
        overridden,
        row("b::New", Some("Workspace"), "ctrl-k", KeymapLayer::Default),
    ];
    let found = m.conflicts(
        "b::New",
        Some("ResourceTable"),
        "ctrl-k",
        &rows,
        Some(&editing),
    );
    assert!(found.is_empty(), "{found:?}");
    assert!(m.conflicts("b::New", None, "  ", &rows, None).is_empty());
}

#[test]
fn the_shipped_stacks_know_the_standard_hierarchy() {
    let m = ContextModel::current();
    assert!(m.can_be_active_together(Some("ResourceTable"), Some("ClusterTab")));
    assert!(m.can_be_active_together(Some("ResourceTable"), Some("Workspace")));
    assert!(!m.can_be_active_together(Some("ResourceTable"), Some("Terminal")));
    assert!(!m.can_be_active_together(Some("ResourceTable"), Some("Catalog")));
    assert!(m.can_be_active_together(Some("Palette"), Some("Picker")));
    assert!(!m.can_be_active_together(Some("Palette"), Some("ClusterTab")));
}

#[test]
fn a_user_binding_of_the_same_keys_in_the_same_context_is_marked_overwritten() {
    let m = model();
    let rows = [
        row(
            "a::Mine",
            Some("ResourceTable&&!Editing"),
            "ctrl-k",
            KeymapLayer::User,
        ),
        row(
            "a::Default",
            Some("ResourceTable && !Editing"),
            "ctrl-k",
            KeymapLayer::Default,
        ),
        row("a::Parent", Some("Workspace"), "ctrl-k", KeymapLayer::User),
        row(
            "a::Longer",
            Some("ResourceTable && !Editing"),
            "ctrl-k ctrl-s",
            KeymapLayer::User,
        ),
    ];
    let found = m.conflicts(
        "b::New",
        Some("ResourceTable && !Editing"),
        "ctrl-k",
        &rows,
        None,
    );
    let overwritten: Vec<_> = found
        .iter()
        .map(|c| (c.row.action.as_str(), c.overwritten))
        .collect();
    assert_eq!(
        overwritten,
        [
            ("a::Mine", true),
            ("a::Default", false),
            ("a::Parent", false),
            ("a::Longer", false),
        ]
    );
}

#[test]
fn an_overwritten_binding_is_gone_whichever_answer_the_user_gives() {
    // What the flag promises: "Keep both" and "Replace them" write the same file, and the other
    // action's binding is not in it.
    let text = r#"[{ "context": "ResourceTable", "bindings": { "ctrl-k": "foo::Action" } }]"#;
    let found = model().conflicts(
        "bar::Action",
        Some("ResourceTable"),
        "ctrl-k",
        &[row(
            "foo::Action",
            Some("ResourceTable"),
            "ctrl-k",
            KeymapLayer::User,
        )],
        None,
    );
    assert_eq!(found.len(), 1);
    assert!(found[0].overwritten, "{found:?}");
    let keep_both = KeymapBindingEdit {
        action: "bar::Action".into(),
        data: None,
        context: Some("ResourceTable".into()),
        keystrokes: "ctrl-k".into(),
        replaces: None,
        unbind: Vec::new(),
    };
    let replace = KeymapBindingEdit {
        unbind: vec![KeymapBindingRef {
            context: Some("ResourceTable".into()),
            keystrokes: "ctrl-k".into(),
            source: KeymapSource::User,
        }],
        ..keep_both.clone()
    };
    let written = |edit: &KeymapBindingEdit| {
        apply_operations(text, &set_binding_operations(edit).expect("a plan")).expect("an edit")
    };
    let kept = written(&keep_both);
    assert_eq!(kept, written(&replace));
    assert!(
        kept.contains("\"ctrl-k\": \"bar::Action\"") && !kept.contains("foo::Action"),
        "{kept}"
    );
}

#[test]
fn a_row_out_of_force_has_no_reference_so_re_recording_it_keeps_the_winning_binding() {
    // The audit's case: the default `cmd-k` -> `x::Old` is overridden by the user's
    // `cmd-k` -> `x::New`. Re-recording the overridden row as `cmd-j` must add `cmd-j` and leave
    // the user's `cmd-k` alone (it used to replace it with `null`).
    let text = r#"[{ "context": "Workspace", "bindings": { "cmd-k": "x::New" } }]"#;
    let mut overridden = row("x::Old", Some("Workspace"), "cmd-k", KeymapLayer::Default);
    overridden.status = RowStatus::Overridden;
    assert!(ref_of_row(&overridden).is_none());
    let mut unbound = row("x::Old", Some("Workspace"), "cmd-k", KeymapLayer::Default);
    unbound.status = RowStatus::UnboundByUser;
    assert!(ref_of_row(&unbound).is_none());
    let mut user_overridden = row("x::Mine", Some("Workspace"), "cmd-k", KeymapLayer::User);
    user_overridden.status = RowStatus::Overridden;
    assert!(ref_of_row(&user_overridden).is_none());
    let active = row("x::New", Some("Workspace"), "cmd-k", KeymapLayer::User);
    assert!(ref_of_row(&active).is_some());

    let edit = KeymapBindingEdit {
        action: "x::Old".into(),
        data: None,
        context: Some("Workspace".into()),
        keystrokes: "cmd-j".into(),
        replaces: ref_of_row(&overridden),
        unbind: Vec::new(),
    };
    let written =
        apply_operations(text, &set_binding_operations(&edit).expect("a plan")).expect("an edit");
    assert!(
        written.contains("\"cmd-k\": \"x::New\"") && written.contains("\"cmd-j\": \"x::Old\""),
        "{written}"
    );
    assert!(!written.contains("null"), "{written}");
}
