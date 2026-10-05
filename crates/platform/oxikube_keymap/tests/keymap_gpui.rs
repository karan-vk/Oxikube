//! GPUI-level tests of the keymap: layering, unbinding, validation, vim flag, hot reload and
//! context scoping, driven through `simulate_keystrokes` so they exercise GPUI's real dispatch.

mod common;

use common::*;
use gpui::{KeyBinding, TestAppContext};
use oxikube_keymap::{
    KeymapLayer, KeymapOptions, KeymapPlatform, KeymapProblem, bindings_for_action_name,
    diagnostics, init_with_dir, init_with_options, init_with_text, reload_user_keymap,
    set_vim_layer,
};

fn mac() -> KeymapOptions {
    KeymapOptions {
        platform: KeymapPlatform::MacOs,
        vim: false,
    }
}

fn install(cx: &mut TestAppContext, options: KeymapOptions, user: &str) {
    cx.update(|cx| init_with_text(user, options, cx));
}

#[gpui::test]
fn embedded_defaults_dispatch_in_any_context(cx: &mut TestAppContext) {
    install(cx, mac(), "");
    let (window, log) = probe(cx, "Pane");
    assert_eq!(press(cx, window, &log, "cmd-shift-p"), ["Toggle"]);
    assert_eq!(press(cx, window, &log, "cmd-q"), ["Quit"]);
}

#[gpui::test]
fn the_default_keymap_follows_the_os(cx: &mut TestAppContext) {
    let linux = KeymapOptions {
        platform: KeymapPlatform::Linux,
        vim: false,
    };
    install(cx, linux, "");
    let (window, log) = probe(cx, "Pane");
    assert_eq!(press(cx, window, &log, "ctrl-q"), ["Quit"]);
    assert!(
        press(cx, window, &log, "cmd-q").is_empty(),
        "cmd-q is macOS only"
    );

    install(cx, mac(), "");
    assert!(press(cx, window, &log, "ctrl-q").is_empty());
    assert_eq!(press(cx, window, &log, "cmd-q"), ["Quit"]);
}

#[gpui::test]
fn a_user_binding_overrides_the_default_for_the_same_key_and_context(cx: &mut TestAppContext) {
    install(
        cx,
        mac(),
        r#"[{"bindings": {"cmd-shift-p": "kmtest::Alpha"}}]"#,
    );
    let (window, log) = probe(cx, "Pane");
    assert_eq!(press(cx, window, &log, "cmd-shift-p"), ["Alpha"]);
    // The rest of the defaults are untouched.
    assert_eq!(press(cx, window, &log, "cmd-q"), ["Quit"]);
}

#[gpui::test]
fn a_user_binding_in_a_deeper_context_overrides_the_default(cx: &mut TestAppContext) {
    install(
        cx,
        mac(),
        r#"[{"context": "Table", "bindings": {"cmd-shift-p": "kmtest::Beta"}}]"#,
    );
    let (table_window, table_log) = probe(cx, "Table");
    assert_eq!(press(cx, table_window, &table_log, "cmd-shift-p"), ["Beta"]);
    let (pane_window, pane_log) = probe(cx, "Pane");
    assert_eq!(press(cx, pane_window, &pane_log, "cmd-shift-p"), ["Toggle"]);
}

#[gpui::test]
fn null_unbinds_a_default(cx: &mut TestAppContext) {
    install(cx, mac(), r#"[{"bindings": {"cmd-q": null}}]"#);
    let (window, log) = probe(cx, "Pane");
    assert!(
        press(cx, window, &log, "cmd-q").is_empty(),
        "cmd-q is unbound"
    );
    assert_eq!(
        press(cx, window, &log, "cmd-shift-p"),
        ["Toggle"],
        "others stay"
    );
    assert!(diagnostics_of(cx).is_empty());
}

#[gpui::test]
fn null_can_be_followed_by_a_new_binding_for_the_same_key(cx: &mut TestAppContext) {
    install(
        cx,
        mac(),
        r#"[{"bindings": {"cmd-q": null}}, {"bindings": {"cmd-q": "kmtest::Gamma"}}]"#,
    );
    let (window, log) = probe(cx, "Pane");
    assert_eq!(press(cx, window, &log, "cmd-q"), ["Gamma"]);
}

#[gpui::test]
fn a_binding_applies_only_in_its_context(cx: &mut TestAppContext) {
    install(
        cx,
        mac(),
        r#"[{"context": "Table && !Editing", "bindings": {"ctrl-a": "kmtest::Alpha"}}]"#,
    );
    let (table, table_log) = probe(cx, "Table");
    assert_eq!(press(cx, table, &table_log, "ctrl-a"), ["Alpha"]);
    let (pane, pane_log) = probe(cx, "Pane");
    assert!(
        press(cx, pane, &pane_log, "ctrl-a").is_empty(),
        "wrong context"
    );
    let (editing, editing_log) = probe(cx, "Table Editing");
    assert!(
        press(cx, editing, &editing_log, "ctrl-a").is_empty(),
        "negated flag"
    );
}

#[gpui::test]
fn key_sequences_and_actions_with_data_work(cx: &mut TestAppContext) {
    install(
        cx,
        mac(),
        r#"[{"bindings": {"ctrl-k ctrl-s": ["kmtest::Scale", {"replicas": 3}]}}]"#,
    );
    let (window, log) = probe(cx, "Pane");
    assert_eq!(press(cx, window, &log, "ctrl-k ctrl-s"), ["Scale(3)"]);
    assert!(diagnostics_of(cx).is_empty());
}

#[gpui::test]
fn an_unknown_action_is_a_diagnostic_and_the_rest_of_the_file_loads(cx: &mut TestAppContext) {
    install(
        cx,
        mac(),
        r#"[{"bindings": {"ctrl-x": "nope::Missing", "ctrl-a": "kmtest::Alpha"}}]"#,
    );
    let (window, log) = probe(cx, "Pane");
    assert_eq!(press(cx, window, &log, "ctrl-a"), ["Alpha"]);
    assert!(press(cx, window, &log, "ctrl-x").is_empty());
    let found = diagnostics_of(cx);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].layer, KeymapLayer::User);
    assert_eq!(found[0].keystrokes.as_deref(), Some("ctrl-x"));
    assert_eq!(
        found[0].problem,
        KeymapProblem::UnknownAction {
            name: "nope::Missing".into()
        }
    );
}

#[gpui::test]
fn bad_keystrokes_data_context_and_binding_values_are_reported_individually(
    cx: &mut TestAppContext,
) {
    install(
        cx,
        mac(),
        r#"[
          {"bindings": {"ctrl-a": "kmtest::Alpha", "not-a-real-modifier-": "kmtest::Beta", "": "kmtest::Beta"}},
          {"bindings": {"ctrl-b": ["kmtest::Scale", {"replicas": "many"}], "ctrl-c": 7}},
          {"context": "Table &&", "bindings": {"ctrl-d": "kmtest::Gamma"}},
          {"bindings": {"ctrl-e": "kmtest::Gamma"}}
        ]"#,
    );
    let (window, log) = probe(cx, "Table");
    assert_eq!(press(cx, window, &log, "ctrl-a"), ["Alpha"]);
    assert_eq!(press(cx, window, &log, "ctrl-e"), ["Gamma"]);
    assert!(
        press(cx, window, &log, "ctrl-d").is_empty(),
        "section with a bad context is skipped"
    );

    let found = diagnostics_of(cx);
    let kinds: Vec<_> = found.iter().map(|d| (d.section, &d.problem)).collect();
    assert!(
        matches!(
            kinds[..],
            [
                (Some(0), KeymapProblem::InvalidKeystrokes { .. }),
                (Some(0), KeymapProblem::InvalidKeystrokes { .. }),
                (Some(1), KeymapProblem::InvalidActionData { .. }),
                (Some(1), KeymapProblem::InvalidBinding { .. }),
                (Some(2), KeymapProblem::InvalidContext { .. }),
            ]
        ),
        "{kinds:#?}"
    );
}

#[gpui::test]
fn malformed_json_keeps_the_previous_keymap(cx: &mut TestAppContext) {
    install(cx, mac(), r#"[{"bindings": {"ctrl-a": "kmtest::Alpha"}}]"#);
    let (window, log) = probe(cx, "Pane");
    assert_eq!(press(cx, window, &log, "ctrl-a"), ["Alpha"]);

    cx.update(|cx| reload_user_keymap(cx, r#"[{"bindings": {"ctrl-a": "kmtest::Beta""#));
    assert_eq!(
        press(cx, window, &log, "ctrl-a"),
        ["Alpha"],
        "previous keymap stays"
    );
    assert_eq!(press(cx, window, &log, "cmd-q"), ["Quit"], "defaults stay");
    let found = diagnostics_of(cx);
    assert!(
        matches!(found[..], [ref d] if matches!(d.problem, KeymapProblem::InvalidFile { .. })),
        "{found:?}"
    );

    // Fixing the file clears the problem and applies the new text.
    cx.update(|cx| reload_user_keymap(cx, r#"[{"bindings": {"ctrl-a": "kmtest::Beta"}}]"#));
    assert_eq!(press(cx, window, &log, "ctrl-a"), ["Beta"]);
    assert!(diagnostics_of(cx).is_empty());
}

#[gpui::test]
fn reload_replaces_bindings_and_unchanged_text_rebinds_nothing(cx: &mut TestAppContext) {
    install(cx, mac(), r#"[{"bindings": {"ctrl-a": "kmtest::Alpha"}}]"#);
    let (window, log) = probe(cx, "Pane");
    let version = |cx: &mut TestAppContext| cx.read(|cx| cx.key_bindings().borrow().version());

    let before = version(cx);
    cx.update(|cx| {
        reload_user_keymap(
            cx,
            "// comment only change\n[{\"bindings\": {\"ctrl-a\": \"kmtest::Alpha\"}}]",
        )
    });
    assert!(version(cx) == before, "equal sections must not rebind");

    cx.update(|cx| reload_user_keymap(cx, r#"[{"bindings": {"ctrl-b": "kmtest::Beta"}}]"#));
    assert!(
        press(cx, window, &log, "ctrl-a").is_empty(),
        "old binding is gone"
    );
    assert_eq!(press(cx, window, &log, "ctrl-b"), ["Beta"]);
}

#[gpui::test]
fn the_vim_layer_applies_only_when_the_flag_is_set(cx: &mut TestAppContext) {
    install(cx, mac(), "");
    let (table, table_log) = probe(cx, "Table");
    assert!(press(cx, table, &table_log, "j").is_empty(), "flag off");

    cx.update(|cx| set_vim_layer(cx, true));
    assert_eq!(press(cx, table, &table_log, "j"), ["SelectNext"]);
    assert_eq!(press(cx, table, &table_log, "k"), ["SelectPrevious"]);
    let (editing, editing_log) = probe(cx, "Table Editing");
    assert!(
        press(cx, editing, &editing_log, "j").is_empty(),
        "typing in a field"
    );
    let (pane, pane_log) = probe(cx, "Pane");
    assert!(press(cx, pane, &pane_log, "j").is_empty(), "wrong context");

    cx.update(|cx| set_vim_layer(cx, false));
    assert!(
        press(cx, table, &table_log, "j").is_empty(),
        "flag off again"
    );
}

#[gpui::test]
fn the_vim_flag_can_be_set_at_init_and_the_user_layer_still_wins(cx: &mut TestAppContext) {
    let options = KeymapOptions { vim: true, ..mac() };
    install(
        cx,
        options,
        r#"[{"context": "Table", "bindings": {"j": "kmtest::Alpha"}}]"#,
    );
    let (table, log) = probe(cx, "Table");
    assert_eq!(press(cx, table, &log, "j"), ["Alpha"], "user beats vim");
    assert_eq!(press(cx, table, &log, "k"), ["SelectPrevious"]);
}

#[gpui::test]
fn bindings_added_by_other_crates_survive_reloads_and_rank_below_the_keymap(
    cx: &mut TestAppContext,
) {
    cx.update(|cx| {
        cx.bind_keys([
            KeyBinding::new("ctrl-f", Gamma, Some("Table")),
            KeyBinding::new("ctrl-g", Gamma, Some("Table")),
        ])
    });
    install(
        cx,
        mac(),
        r#"[{"context": "Table", "bindings": {"ctrl-g": "kmtest::Alpha"}}]"#,
    );
    let (window, log) = probe(cx, "Table");
    assert_eq!(
        press(cx, window, &log, "ctrl-f"),
        ["Gamma"],
        "foreign binding kept"
    );
    assert_eq!(
        press(cx, window, &log, "ctrl-g"),
        ["Alpha"],
        "keymap outranks it"
    );

    cx.update(|cx| reload_user_keymap(cx, "[]"));
    assert_eq!(
        press(cx, window, &log, "ctrl-f"),
        ["Gamma"],
        "still there after a reload"
    );
    assert_eq!(press(cx, window, &log, "ctrl-g"), ["Gamma"]);
}

#[gpui::test]
fn bindings_for_an_action_are_listed_strongest_first(cx: &mut TestAppContext) {
    install(
        cx,
        mac(),
        r#"[{"context": "Table", "bindings": {"ctrl-t": "palette::Toggle"}}]"#,
    );
    let listed = cx.read(|cx| bindings_for_action_name(cx, "palette::Toggle", None));
    assert_eq!(listed.len(), 2, "{listed:?}");
    assert_eq!(listed[0].keystrokes_text(), "ctrl-t");
    assert_eq!(listed[0].context.as_deref(), Some("Table"));
    assert_eq!(listed[0].layer, Some(KeymapLayer::User));
    assert_eq!(listed[1].keystrokes_text(), "cmd-shift-p");
    assert_eq!(listed[1].layer, Some(KeymapLayer::Default));

    cx.update(|cx| reload_user_keymap(cx, r#"[{"bindings": {"cmd-shift-p": null}}]"#));
    let listed = cx.read(|cx| bindings_for_action_name(cx, "palette::Toggle", None));
    assert!(
        listed.is_empty(),
        "an unbound default is not listed: {listed:?}"
    );
    assert!(
        cx.read(|cx| bindings_for_action_name(cx, "nope::Missing", None))
            .is_empty()
    );
}

#[gpui::test]
fn the_user_file_is_read_from_the_config_dir(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    cx.update(|cx| init_with_dir(dir.path(), mac(), cx));
    let (window, log) = probe(cx, "Pane");
    assert_eq!(
        press(cx, window, &log, "cmd-q"),
        ["Quit"],
        "a missing file is fine"
    );

    std::fs::write(
        dir.path().join("keymap.json"),
        r#"[{"bindings": {"cmd-q": "kmtest::Alpha"}}]"#,
    )
    .unwrap();
    cx.update(|cx| init_with_dir(dir.path(), mac(), cx));
    assert_eq!(press(cx, window, &log, "cmd-q"), ["Alpha"]);
}

#[gpui::test]
fn an_unreadable_user_file_is_reported_and_a_later_fix_applies(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    // Invalid UTF-8, as a UTF-16 save from Notepad is.
    std::fs::write(
        dir.path().join("keymap.json"),
        [0xFF, 0xFE, b'[', 0, b']', 0],
    )
    .unwrap();
    cx.update(|cx| init_with_dir(dir.path(), mac(), cx));
    let (window, log) = probe(cx, "Pane");
    assert_eq!(
        press(cx, window, &log, "cmd-q"),
        ["Quit"],
        "defaults stay bound"
    );
    let found = diagnostics_of(cx);
    assert!(
        matches!(found.as_slice(), [d] if d.layer == KeymapLayer::User
            && matches!(d.problem, KeymapProblem::Unreadable { .. })),
        "{found:?}"
    );
    assert!(
        found[0]
            .to_string()
            .starts_with("keymap.json could not be read")
    );

    // What the watcher delivers once the file is saved as UTF-8.
    cx.update(|cx| reload_user_keymap(cx, r#"[{"bindings": {"cmd-q": "kmtest::Alpha"}}]"#));
    assert_eq!(press(cx, window, &log, "cmd-q"), ["Alpha"]);
    assert!(diagnostics_of(cx).is_empty());
}

#[gpui::test]
fn init_with_the_default_options_installs_the_current_os_keymap(cx: &mut TestAppContext) {
    // `init_with_options` reads the real config dir, so point it at an empty temp dir.
    let dir = tempfile::tempdir().unwrap();
    cx.update(|cx| init_with_dir(dir.path(), KeymapOptions::default(), cx));
    let _ = init_with_options; // exercised by the binary; compiled here to keep the API honest
    let (window, log) = probe(cx, "Pane");
    let quit = if cfg!(target_os = "macos") {
        "cmd-q"
    } else {
        "ctrl-q"
    };
    assert_eq!(press(cx, window, &log, quit), ["Quit"]);
}

fn diagnostics_of(cx: &mut TestAppContext) -> Vec<oxikube_keymap::KeymapDiagnostic> {
    cx.read(diagnostics)
}
