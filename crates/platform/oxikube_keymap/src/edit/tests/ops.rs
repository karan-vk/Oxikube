//! The edit operations on `keymap.json` text with comments: the output keeps the comments and
//! passes the validator.

use oxikube_domain::command::{KeymapBindingEdit, KeymapBindingRef, KeymapSource};

use crate::edit::{
    BindingLocation, EditError, KeybindTarget, KeybindUpdateOperation, apply_operations,
    preview_edit, remove_binding_operations, set_binding_operations,
};
use crate::file::{KeymapAction, parse_keymap};
use crate::known::KnownNames;
use crate::layer::KeymapLayer;
use crate::registry::ActionRegistry;

const USER: &str = r#"// My keys.
[
  // Table tweaks.
  {
    "context": "ResourceTable",
    "bindings": {
      "ctrl-j": "table::SelectNext", // down
      "ctrl-k": "table::SelectPrevious"
    }
  },
  /* Everywhere. */
  {
    "bindings": {
      "ctrl-alt-p": "palette::Toggle"
    }
  }
]
"#;

fn actions() -> ActionRegistry {
    ActionRegistry::from_names([
        "table::SelectNext",
        "table::SelectPrevious",
        "palette::Toggle",
        "workspace::CloseActiveItem",
    ])
}

fn action(name: &str) -> KeymapAction {
    KeymapAction::Action {
        name: name.to_owned(),
        data: None,
    }
}

fn target(context: Option<&str>, keys: &str, name: &str) -> KeybindTarget {
    KeybindTarget {
        context: context.map(str::to_owned),
        keystrokes: keys.to_owned(),
        action: action(name),
    }
}

fn at(context: Option<&str>, keys: &str, layer: KeymapLayer) -> BindingLocation {
    BindingLocation {
        context: context.map(str::to_owned),
        keystrokes: keys.to_owned(),
        layer,
    }
}

/// Apply, then check the comments survived and the result validates cleanly.
fn edit(text: &str, operations: &[KeybindUpdateOperation]) -> String {
    let preview = preview_edit(text, operations, &actions(), &KnownNames::current()).unwrap();
    assert!(
        preview.diagnostics.is_empty(),
        "{:?}\n{}",
        preview.diagnostics,
        preview.text
    );
    preview.text
}

/// The bindings of the sections with `context`, as sorted `keys=value` lines. (A new key goes
/// first in its section, as Zed's JSON edit inserts it; the order inside a section only matters
/// for duplicates, which the edits never create.)
fn bindings(text: &str, context: Option<&str>) -> Vec<String> {
    let parsed = parse_keymap(text, KeymapLayer::User).unwrap();
    let mut out: Vec<String> = parsed
        .sections
        .iter()
        .filter(|(_, s)| s.context_expr() == context)
        .flat_map(|(_, s)| s.bindings.iter().map(|(k, v)| format!("{k}={v}")))
        .collect();
    out.sort();
    out
}

fn assert_comments_kept(text: &str) {
    for comment in [
        "// My keys.",
        "// Table tweaks.",
        "// down",
        "/* Everywhere. */",
    ] {
        assert!(text.contains(comment), "{comment} lost:\n{text}");
    }
}

#[test]
fn add_to_an_existing_section_keeps_one_section_per_context() {
    let out = edit(
        USER,
        &[KeybindUpdateOperation::Add(target(
            Some("ResourceTable"),
            "ctrl-w",
            "workspace::CloseActiveItem",
        ))],
    );
    assert_comments_kept(&out);
    assert_eq!(
        bindings(&out, Some("ResourceTable")),
        [
            "ctrl-j=\"table::SelectNext\"",
            "ctrl-k=\"table::SelectPrevious\"",
            "ctrl-w=\"workspace::CloseActiveItem\""
        ]
    );
    assert_eq!(
        parse_keymap(&out, KeymapLayer::User)
            .unwrap()
            .sections
            .len(),
        2
    );
}

#[test]
fn add_to_a_new_context_appends_a_section() {
    let out = edit(
        USER,
        &[KeybindUpdateOperation::Add(target(
            Some("Terminal"),
            "ctrl-shift-w",
            "workspace::CloseActiveItem",
        ))],
    );
    assert_comments_kept(&out);
    let parsed = parse_keymap(&out, KeymapLayer::User).unwrap();
    assert_eq!(parsed.sections.len(), 3);
    assert_eq!(
        bindings(&out, Some("Terminal")),
        ["ctrl-shift-w=\"workspace::CloseActiveItem\""]
    );
}

#[test]
fn add_to_an_empty_or_blank_file_creates_the_list() {
    for text in ["", "[]", "// nothing yet\n[\n]\n"] {
        let out = edit(
            text,
            &[KeybindUpdateOperation::Add(target(
                None,
                "cmd-shift-k",
                "palette::Toggle",
            ))],
        );
        assert_eq!(
            bindings(&out, None),
            ["cmd-shift-k=\"palette::Toggle\""],
            "{out}"
        );
        if text.contains("//") {
            assert!(out.contains("// nothing yet"), "{out}");
        }
    }
}

#[test]
fn adding_keys_spelled_another_way_replaces_the_binding_instead_of_duplicating_it() {
    let text = r#"[{ "bindings": { "shift-ctrl-q": "palette::Toggle" } }]"#;
    let out = edit(
        text,
        &[KeybindUpdateOperation::Add(target(
            None,
            "ctrl-shift-q",
            "workspace::CloseActiveItem",
        ))],
    );
    assert_eq!(
        bindings(&out, None),
        ["ctrl-shift-q=\"workspace::CloseActiveItem\""]
    );
}

#[test]
fn replace_edits_a_user_binding_in_place() {
    let out = edit(
        USER,
        &[KeybindUpdateOperation::Replace {
            target: at(Some("ResourceTable"), "ctrl-j", KeymapLayer::User),
            with: target(Some("ResourceTable"), "ctrl-n", "table::SelectNext"),
        }],
    );
    assert_comments_kept(&out);
    assert_eq!(
        bindings(&out, Some("ResourceTable")),
        [
            "ctrl-k=\"table::SelectPrevious\"",
            "ctrl-n=\"table::SelectNext\""
        ],
        "{out}"
    );
    // Renamed where it was, its trailing comment with it.
    assert!(
        out.contains("\"ctrl-n\": \"table::SelectNext\", // down"),
        "{out}"
    );
}

#[test]
fn replace_into_another_context_moves_the_binding() {
    let out = edit(
        USER,
        &[KeybindUpdateOperation::Replace {
            target: at(Some("ResourceTable"), "ctrl-k", KeymapLayer::User),
            with: target(None, "ctrl-k", "table::SelectPrevious"),
        }],
    );
    assert_eq!(
        bindings(&out, Some("ResourceTable")),
        ["ctrl-j=\"table::SelectNext\""]
    );
    assert_eq!(
        bindings(&out, None),
        [
            "ctrl-alt-p=\"palette::Toggle\"",
            "ctrl-k=\"table::SelectPrevious\""
        ]
    );
}

#[test]
fn replacing_a_default_overrides_it_and_unbinds_its_old_keys() {
    let out = edit(
        USER,
        &[KeybindUpdateOperation::Replace {
            target: at(Some("Workspace"), "cmd-w", KeymapLayer::Default),
            with: target(
                Some("Workspace"),
                "cmd-shift-w",
                "workspace::CloseActiveItem",
            ),
        }],
    );
    assert_comments_kept(&out);
    assert_eq!(
        bindings(&out, Some("Workspace")),
        ["cmd-shift-w=\"workspace::CloseActiveItem\"", "cmd-w=null"]
    );
    // Same keys, same context: the override alone outranks the default.
    let same = edit(
        USER,
        &[KeybindUpdateOperation::Replace {
            target: at(Some("Workspace"), "cmd-w", KeymapLayer::Default),
            with: target(Some("Workspace"), "cmd-w", "palette::Toggle"),
        }],
    );
    assert_eq!(
        bindings(&same, Some("Workspace")),
        ["cmd-w=\"palette::Toggle\""]
    );
}

#[test]
fn remove_deletes_a_user_binding_and_an_emptied_section() {
    let out = edit(
        USER,
        &[KeybindUpdateOperation::Remove {
            target: at(Some("ResourceTable"), "ctrl-k", KeymapLayer::User),
        }],
    );
    assert_comments_kept(&out);
    assert_eq!(
        bindings(&out, Some("ResourceTable")),
        ["ctrl-j=\"table::SelectNext\""]
    );

    let out = edit(
        USER,
        &[KeybindUpdateOperation::Remove {
            target: at(None, "ctrl-alt-p", KeymapLayer::User),
        }],
    );
    assert!(out.contains("// Table tweaks."), "{out}");
    let parsed = parse_keymap(&out, KeymapLayer::User).unwrap();
    assert_eq!(parsed.sections.len(), 1, "the empty section went:\n{out}");
}

#[test]
fn remove_of_a_default_unbinds_it_with_null_in_the_user_file() {
    let out = edit(
        USER,
        &[KeybindUpdateOperation::Remove {
            target: at(Some("ResourceTable"), "y", KeymapLayer::Default),
        }],
    );
    assert_comments_kept(&out);
    assert_eq!(
        bindings(&out, Some("ResourceTable")),
        [
            "ctrl-j=\"table::SelectNext\"",
            "ctrl-k=\"table::SelectPrevious\"",
            "y=null"
        ]
    );
}

#[test]
fn a_missing_user_binding_and_a_broken_file_are_errors_not_edits() {
    let missing = apply_operations(
        USER,
        &[KeybindUpdateOperation::Remove {
            target: at(Some("Terminal"), "ctrl-k", KeymapLayer::User),
        }],
    );
    assert!(
        matches!(missing, Err(EditError::NotFound { .. })),
        "{missing:?}"
    );
    let broken = apply_operations(
        "{ \"not\": \"a list\" }",
        &[KeybindUpdateOperation::Add(target(
            None,
            "a",
            "palette::Toggle",
        ))],
    );
    assert!(
        matches!(broken, Err(EditError::InvalidFile(_))),
        "{broken:?}"
    );
}

#[test]
fn an_edit_that_adds_an_error_is_not_writable_but_old_errors_do_not_block() {
    let bad = preview_edit(
        USER,
        &[KeybindUpdateOperation::Add(target(
            None,
            "ctrl-q",
            "nope::Missing",
        ))],
        &actions(),
        &KnownNames::current(),
    )
    .unwrap();
    assert!(!bad.is_writable());
    assert_eq!(bad.introduced.len(), 1, "{:?}", bad.introduced);

    let already = r#"[{ "bindings": { "ctrl-x": "nope::Old" } }]"#;
    let fine = preview_edit(
        already,
        &[KeybindUpdateOperation::Add(target(
            None,
            "ctrl-q",
            "palette::Toggle",
        ))],
        &actions(),
        &KnownNames::current(),
    )
    .unwrap();
    assert!(fine.is_writable(), "{:?}", fine.introduced);
    assert!(
        !fine.diagnostics.is_empty(),
        "the old error is still reported"
    );
}

#[test]
fn crlf_and_tab_indented_files_keep_their_style() {
    let crlf = "[\r\n\t{\r\n\t\t\"bindings\": {\r\n\t\t\t\"ctrl-j\": \"table::SelectNext\"\r\n\t\t}\r\n\t}\r\n]\r\n";
    let out = edit(
        crlf,
        &[KeybindUpdateOperation::Add(target(
            None,
            "ctrl-k",
            "table::SelectPrevious",
        ))],
    );
    assert_eq!(
        bindings(&out, None),
        [
            "ctrl-j=\"table::SelectNext\"",
            "ctrl-k=\"table::SelectPrevious\""
        ]
    );
}

#[test]
fn set_binding_plans_the_binding_then_the_replaced_conflicts() {
    let edit = KeymapBindingEdit {
        action: "palette::Toggle".into(),
        data: Some("{\"query\":\"pod\"}".into()),
        context: Some("  ResourceTable ".into()),
        keystrokes: "g  g".into(),
        replaces: None,
        unbind: vec![
            KeymapBindingRef {
                context: Some("Workspace".into()),
                keystrokes: "g".into(),
                source: KeymapSource::Default,
            },
            // The new binding already outranks this one: no `null` for it.
            KeymapBindingRef {
                context: Some("ResourceTable".into()),
                keystrokes: "g g".into(),
                source: KeymapSource::Default,
            },
        ],
    };
    let operations = set_binding_operations(&edit).unwrap();
    assert_eq!(operations.len(), 2, "{operations:?}");
    let KeybindUpdateOperation::Add(added) = &operations[0] else {
        panic!("{operations:?}");
    };
    assert_eq!(added.context.as_deref(), Some("ResourceTable"));
    assert_eq!(added.keystrokes, "g g");
    let out = apply_operations("", &operations).unwrap();
    assert_eq!(bindings(&out, Some("Workspace")), ["g=null"]);
    assert_eq!(
        bindings(&out, Some("ResourceTable")),
        ["g g=[\"palette::Toggle\",{\"query\":\"pod\"}]"]
    );

    let blank = KeymapBindingEdit {
        keystrokes: " ".into(),
        ..edit.clone()
    };
    assert!(set_binding_operations(&blank).is_err());
    let bad_data = KeymapBindingEdit {
        data: Some("{".into()),
        ..edit
    };
    assert!(set_binding_operations(&bad_data).is_err());

    let remove = remove_binding_operations(&KeymapBindingRef {
        context: None,
        keystrokes: "ctrl-alt-p".into(),
        source: KeymapSource::User,
    });
    let out = apply_operations(USER, &remove).unwrap();
    assert!(bindings(&out, None).is_empty(), "{out}");
}

#[test]
fn a_new_key_goes_above_the_first_binding_with_its_indentation() {
    let text = "[\n  {\n    \"context\": \"Workspace\",\n    \"bindings\": {\n      \"ctrl-j\": \"table::SelectNext\"\n    }\n  }\n]\n";
    let out = edit(
        text,
        &[KeybindUpdateOperation::Add(target(
            Some("Workspace"),
            "ctrl-k",
            "table::SelectPrevious",
        ))],
    );
    assert_eq!(
        out,
        "[\n  {\n    \"context\": \"Workspace\",\n    \"bindings\": {\n      \"ctrl-k\": \"table::SelectPrevious\",\n      \"ctrl-j\": \"table::SelectNext\"\n    }\n  }\n]\n"
    );
    let inline = r#"[{ "context": "Workspace", "bindings": { "ctrl-j": "table::SelectNext" } }]"#;
    let out = edit(
        inline,
        &[KeybindUpdateOperation::Add(target(
            Some("Workspace"),
            "ctrl-k",
            "table::SelectPrevious",
        ))],
    );
    assert_eq!(
        out,
        r#"[{ "context": "Workspace", "bindings": { "ctrl-k": "table::SelectPrevious", "ctrl-j": "table::SelectNext" } }]"#
    );
    // An empty section of the context is passed over for a new one.
    let empty = "[\n  {\n    \"context\": \"Workspace\"\n  }\n]\n";
    let out = edit(
        empty,
        &[KeybindUpdateOperation::Add(target(
            Some("Workspace"),
            "ctrl-k",
            "table::SelectPrevious",
        ))],
    );
    assert_eq!(
        parse_keymap(&out, KeymapLayer::User)
            .unwrap()
            .sections
            .len(),
        2,
        "{out}"
    );
}
