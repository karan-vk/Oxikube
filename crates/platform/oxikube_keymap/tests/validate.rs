//! The pure validator (E21-S03): unknown actions and contexts with positions, severities and
//! suggestions, duplicate keystrokes, and the shipped keymaps staying free of warnings.

use oxikube_assets::{KeymapPlatform, default_keymap, initial_user_keymap_content, vim_keymap};
use oxikube_keymap::{
    ActionRegistry, KeymapProblem, KnownNames, Severity, diagnostics::KeymapDiagnostic, validate,
};

fn actions() -> ActionRegistry {
    ActionRegistry::from_names([
        "table::SelectNext",
        "table::SelectPrevious",
        "palette::Toggle",
        "workspace::CloseActiveItem",
    ])
}

fn check(text: &str) -> Vec<KeymapDiagnostic> {
    validate(text, &actions(), &KnownNames::current())
}

#[test]
fn a_clean_file_and_blank_text_have_no_problems() {
    assert!(check("").is_empty());
    assert!(check("  // only a comment\n[]").is_empty());
    let clean = r#"[
      {
        "context": "ResourceTable && !Editing && kind == Pod",
        "bindings": { "j": "table::SelectNext", "k": ["table::SelectPrevious", {}], "x": null }
      }
    ]"#;
    assert!(check(clean).is_empty(), "{:?}", check(clean));
}

#[test]
fn an_unknown_action_is_an_error_with_its_line_column_span_and_the_nearest_name() {
    let text = "[\n  {\n    \"bindings\": {\n      \"j\": \"table::SelctNext\"\n    }\n  }\n]";
    let found = check(text);
    assert_eq!(found.len(), 1, "{found:?}");
    let d = &found[0];
    assert_eq!(d.severity(), Severity::Error);
    assert_eq!(d.code(), "keymap/unknown-action");
    assert_eq!((d.line, d.column), (Some(4), Some(7)));
    // The span is the binding's key, `"j"`.
    assert_eq!(&text[d.span.clone().unwrap()], "\"j\"");
    assert_eq!(
        d.problem,
        KeymapProblem::UnknownAction {
            name: "table::SelctNext".into(),
            suggestion: Some("table::SelectNext".into()),
        }
    );
    assert!(d.message().contains("did you mean `table::SelectNext`?"));
}

#[test]
fn an_unknown_context_is_a_warning_with_the_nearest_known_name() {
    let text = "[{\"context\": \"ResourceTabel && !Editing\", \"bindings\": {\"j\": \"table::SelectNext\"}}]";
    let found = check(text);
    assert_eq!(found.len(), 1, "{found:?}");
    let d = &found[0];
    assert_eq!(d.severity(), Severity::Warning);
    assert_eq!(d.code(), "keymap/unknown-context");
    assert_eq!(
        &text[d.span.clone().unwrap()],
        "\"ResourceTabel && !Editing\""
    );
    assert_eq!(
        d.problem,
        KeymapProblem::UnknownContext {
            name: "ResourceTabel".into(),
            suggestion: Some("ResourceTable".into()),
        }
    );
    // A context nothing resembles still warns, without a suggestion.
    let found = check("[{\"context\": \"ExtensionView\", \"bindings\": {}}]");
    assert!(matches!(
        &found[..],
        [d] if matches!(&d.problem, KeymapProblem::UnknownContext { suggestion: None, .. })
    ));
}

#[test]
fn a_registered_context_is_known() {
    let text = "[{\"context\": \"S03RegisteredView && Editing\", \"bindings\": {}}]";
    assert_eq!(check(text).len(), 1);
    oxikube_keymap::register_context("S03RegisteredView");
    assert!(check(text).is_empty());
}

#[test]
fn a_broken_context_predicate_is_an_error() {
    let found = check("[{\"context\": \"ResourceTable &&\", \"bindings\": {}}]");
    assert!(matches!(
        &found[..],
        [d] if matches!(d.problem, KeymapProblem::InvalidContext { .. }) && d.severity() == Severity::Error
    ));
}

#[test]
fn keystrokes_and_binding_values_are_checked() {
    let text = r#"[{"bindings": {
      "ctrl-bogus-x": "table::SelectNext",
      "": "table::SelectNext",
      "j": 7,
      "k": ["table::SelectNext", {}, 3]
    }}]"#;
    let codes: Vec<_> = check(text).iter().map(KeymapDiagnostic::code).collect();
    assert_eq!(
        codes,
        [
            "keymap/invalid-keystrokes",
            "keymap/invalid-keystrokes",
            "keymap/invalid-binding",
            "keymap/invalid-binding",
        ]
    );
}

#[test]
fn a_keystroke_written_twice_in_a_section_is_a_warning_on_the_later_one() {
    let text = "[{\"bindings\": {\n  \"j\": \"table::SelectNext\",\n  \"k\": null,\n  \"j\": \"table::SelectPrevious\"\n}}]";
    let found = check(text);
    assert_eq!(found.len(), 1, "{found:?}");
    let d = &found[0];
    assert_eq!(d.severity(), Severity::Warning);
    assert_eq!(d.line, Some(4));
    assert_eq!(&text[d.span.clone().unwrap()], "\"j\"");
    assert_eq!(
        d.problem,
        KeymapProblem::DuplicateKeystrokes {
            keystrokes: "j".into(),
            first_line: 2
        }
    );
    // The same key in two sections is a normal override, not a duplicate.
    let two = "[{\"bindings\": {\"j\": null}}, {\"bindings\": {\"j\": null}}]";
    assert!(check(two).is_empty());
}

#[test]
fn syntax_errors_and_non_lists_are_reported_with_a_position() {
    let text = "[\n  {\"bindings\": {\"j\": }}\n]";
    let found = check(text);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].code(), "keymap/invalid-file");
    assert_eq!(found[0].line, Some(2));
    assert!(found[0].column.is_some() && found[0].span.is_some());
    let found = check("{}");
    assert_eq!(found[0].code(), "keymap/invalid-file");
}

#[test]
fn diagnostics_come_in_file_order() {
    let text = r#"[
      {"context": "Nope", "bindings": {"j": "table::Missing"}},
      {"bindings": {"k": "table::SelectNext", "k": null}}
    ]"#;
    let lines: Vec<_> = check(text).iter().map(|d| d.line).collect();
    assert_eq!(lines, [Some(2), Some(2), Some(3)]);
}

/// The shipped keymaps (and the template a new user file starts from) must not trip the
/// context checks: a warning in our own defaults would be a validator or registry bug.
#[test]
fn the_shipped_keymaps_use_only_known_contexts_and_have_no_duplicates() {
    let known = KnownNames::current();
    // Actions from other crates are not linked here: only action problems are expected.
    let none = ActionRegistry::from_names([]);
    for platform in [
        KeymapPlatform::MacOs,
        KeymapPlatform::Linux,
        KeymapPlatform::Windows,
    ] {
        check_shipped(default_keymap(platform), &none, &known);
    }
    check_shipped(vim_keymap(), &none, &known);
    check_shipped(initial_user_keymap_content(), &none, &known);
}

fn check_shipped(text: &str, none: &ActionRegistry, known: &KnownNames) {
    let found: Vec<_> = validate(text, none, known)
        .into_iter()
        .filter(|d| !matches!(d.problem, KeymapProblem::UnknownAction { .. }))
        .collect();
    assert!(found.is_empty(), "{found:#?}");
}
