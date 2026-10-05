//! The embedded keymaps are well formed, the action registry reflects what is declared, and a
//! reload costs little.

// Declares the test actions the registry tests look for.
mod common;

use std::time::Instant;

use gpui::{KeyBindingContextPredicate, Keystroke, TestAppContext};
use oxikube_keymap::file::parse_keymap;
use oxikube_keymap::{
    ActionRegistry, KeymapLayer, KeymapOptions, KeymapPlatform, init_with_text, reload_user_keymap,
};

/// Every embedded file parses, and every context expression and keystroke in it is valid,
/// whether or not the owning crate's actions are linked into this test binary (embedded
/// bindings with unregistered actions are skipped, so the build pass alone cannot see them).
#[test]
fn embedded_keymaps_are_well_formed() {
    let embedded = [
        (
            KeymapLayer::Default,
            oxikube_assets::default_keymap(KeymapPlatform::MacOs),
        ),
        (
            KeymapLayer::Default,
            oxikube_assets::default_keymap(KeymapPlatform::Linux),
        ),
        (
            KeymapLayer::Default,
            oxikube_assets::default_keymap(KeymapPlatform::Windows),
        ),
        (KeymapLayer::Vim, oxikube_assets::vim_keymap()),
        (
            KeymapLayer::User,
            oxikube_assets::initial_user_keymap_content(),
        ),
    ];
    for (layer, text) in embedded {
        let parsed = parse_keymap(text, layer).expect("embedded keymap parses");
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        for (_, section) in &parsed.sections {
            if let Some(context) = section.context_expr() {
                KeyBindingContextPredicate::parse(context)
                    .unwrap_or_else(|err| panic!("context `{context}`: {err}"));
            }
            for (keystrokes, value) in &section.bindings {
                assert!(keystrokes.split_whitespace().next().is_some());
                for keystroke in keystrokes.split_whitespace() {
                    Keystroke::parse(keystroke)
                        .unwrap_or_else(|err| panic!("`{keystrokes}`: {err}"));
                }
                let action = oxikube_keymap::KeymapAction::from_json(value).unwrap();
                if let oxikube_keymap::KeymapAction::Action { name, .. } = action {
                    assert!(
                        oxikube_keymap::registry::namespace_of(&name) != "",
                        "`{name}` must be namespaced"
                    );
                }
            }
        }
    }
}

#[test]
fn the_vim_layer_is_scoped_to_contexts_where_typing_is_not_text_entry() {
    let parsed = parse_keymap(oxikube_assets::vim_keymap(), KeymapLayer::Vim).unwrap();
    for (_, section) in parsed.sections {
        let context = section
            .context_expr()
            .expect("vim bindings are never global");
        assert!(context.contains("!Editing"), "{context}");
    }
}

#[test]
fn the_current_os_has_a_keymap() {
    let current = KeymapPlatform::current();
    assert_eq!(current == KeymapPlatform::MacOs, cfg!(target_os = "macos"));
    assert_eq!(
        current == KeymapPlatform::Windows,
        cfg!(target_os = "windows")
    );
}

#[gpui::test]
fn the_registry_groups_declared_actions_by_namespace(cx: &mut TestAppContext) {
    let registry = cx.read(ActionRegistry::from_app);
    assert!(registry.contains("kmtest::Alpha"));
    assert!(registry.contains("kmtest::Scale"));
    assert!(!registry.contains("kmtest::Missing"));
    assert_eq!(
        registry.names_in("kmtest"),
        [
            "kmtest::Alpha",
            "kmtest::Beta",
            "kmtest::Gamma",
            "kmtest::Scale"
        ]
    );
    let namespaces: Vec<_> = registry.namespaces().collect();
    assert!(namespaces.contains(&"kmtest") && namespaces.contains(&"table"));
    assert!(
        namespaces.windows(2).all(|w| w[0] < w[1]),
        "sorted: {namespaces:?}"
    );
    assert!(registry.names_in("nope").is_empty());
}

#[gpui::test]
fn an_action_that_is_a_declared_command_maps_to_it(cx: &mut TestAppContext) {
    let registry = cx.read(ActionRegistry::from_app);
    // `palette::Toggle` is both a registered action (declared by the test) and a Command.
    assert!(registry.contains("palette::Toggle"));
    let meta = ActionRegistry::command("palette::Toggle").unwrap();
    assert_eq!(meta.id.tool_name(), "app.palette_toggle");
    assert!(ActionRegistry::command("kmtest::Alpha").is_none());
    // Commands nobody declared an action for are listed so start-up can warn about them.
    let missing = registry.commands_without_action();
    assert!(!missing.iter().any(|id| id.as_str() == "palette::Toggle"));
    assert!(missing.iter().any(|id| id.as_str() == "pod::Delete"));
}

#[gpui::test]
fn data_action_names_and_build_errors_are_values_not_panics(cx: &mut TestAppContext) {
    use oxikube_keymap::registry::BuildActionError;
    cx.read(|cx| {
        assert!(
            ActionRegistry::build(
                cx,
                "kmtest::Scale",
                Some(serde_json::json!({"replicas": 2}))
            )
            .is_ok()
        );
        assert_eq!(
            ActionRegistry::build(cx, "kmtest::Nope", None).err(),
            Some(BuildActionError::Unknown)
        );
        assert!(matches!(
            ActionRegistry::build(cx, "kmtest::Scale", None).err(),
            Some(BuildActionError::InvalidData(_))
        ));
    });
}

/// A reload of a large user file stays far below one frame. Prints the per-section cost; the
/// bound is deliberately loose so a slow CI machine does not flake (a debug build here is
/// still ~100x under it).
#[gpui::test]
#[allow(clippy::print_stdout)]
fn reload_cost_is_small(cx: &mut TestAppContext) {
    let sections = 50;
    // Valid, distinct keys: `ctrl-alt-<letter>` (13 letters) for the plain bindings.
    let letters: Vec<char> = ('a'..='l').collect();
    let per_section = letters.len();
    let mut text = String::from("[\n");
    for s in 0..sections {
        text.push_str("{\"context\": \"Table && selection == one\", \"bindings\": {");
        for (b, letter) in letters.iter().enumerate() {
            let action = ["kmtest::Alpha", "kmtest::Beta", "kmtest::Gamma"][b % 3];
            // A different modifier set per section keeps every binding in the file distinct.
            let mods = ["ctrl-alt", "ctrl-shift", "alt-shift", "ctrl-alt-shift"][s % 4];
            text.push_str(&format!("\"{mods}-{letter}\": \"{action}\","));
        }
        text.push_str("\"ctrl-k ctrl-s\": [\"kmtest::Scale\", {\"replicas\": 2}]}},\n");
    }
    text.push(']');

    let init_start = Instant::now();
    cx.update(|cx| init_with_text("", KeymapOptions::default(), cx));
    println!(
        "init with the embedded defaults: {:?}",
        init_start.elapsed()
    );
    let start = Instant::now();
    cx.update(|cx| reload_user_keymap(cx, &text));
    let elapsed = start.elapsed();
    let per = elapsed / sections as u32;
    println!(
        "reload of {sections} sections x {} bindings: {elapsed:?} ({per:?} per section)",
        per_section + 1
    );
    // Every binding was accepted: this measures real `KeyBinding` construction and layering,
    // not the cheap rejection path.
    let problems = cx.read(oxikube_keymap::diagnostics);
    assert!(problems.is_empty(), "{problems:?}");
    let bound = cx.read(|cx| {
        cx.key_bindings()
            .borrow()
            .bindings()
            .filter(|b| KeymapLayer::from_meta(b.meta()) == Some(KeymapLayer::User))
            .count()
    });
    assert_eq!(bound, sections * (per_section + 1));
    assert!(per.as_millis() < 5, "{per:?} per section");
}
