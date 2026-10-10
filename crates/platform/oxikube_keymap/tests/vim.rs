//! The vim base keymap (E11-S09): `vim.json` layered by `base_keymap`, resolved per context.
//!
//! The shipped `default-*.json` and `vim.json` are installed for real and the actions they name
//! are declared here under their real names (the crates that own them are not linked into this
//! test), so a binding is live exactly as in the app. Each test asks what a key does for a
//! context stack written as data (`oxikube_keymap::resolve`), the question GPUI answers when a
//! key is pressed. `oxikube_resources_ui` presses the same keys in a real table, and the binary's
//! `startup` tests check the real crates register every action `vim.json` names.

use std::collections::BTreeSet;

use gpui::{Action, TestAppContext, UpdateGlobal as _, actions};
use oxikube_keymap::file::{KeymapSection, parse_keymap};
use oxikube_keymap::{
    ActionRegistry, KeymapAction, KeymapLayer, KeymapOptions, KeymapPlatform, KeymapSettings,
    Resolution, init_with_text, parse_stack, reload_user_keymap, resolve,
};
use oxikube_settings::SettingsStore;
use schemars::JsonSchema;
use serde::Deserialize;

actions!(
    resource_table,
    [
        SelectNext,
        SelectPrevious,
        SelectFirst,
        SelectLast,
        SelectPageDown,
        SelectPageUp,
        SelectHalfPageDown,
        SelectHalfPageUp,
        DeleteSelected,
        CopyName,
        FocusFilter,
        ViewYaml,
        ViewDescribe
    ]
);
actions!(palette, [OpenJump]);
actions!(help, [Show]);

/// The detail drawer's actions, in a module of their own: their names repeat the table's.
mod drawer {
    use super::*;

    actions!(resource_detail, [Close, SelectNext, SelectPrevious]);

    #[derive(Clone, PartialEq, Deserialize, JsonSchema, Action)]
    #[action(namespace = resource_detail)]
    pub struct ShowTab {
        pub index: u8,
    }
}

const MAC: KeymapPlatform = KeymapPlatform::MacOs;
const LINUX: KeymapPlatform = KeymapPlatform::Linux;

fn options(platform: KeymapPlatform, vim: bool) -> KeymapOptions {
    KeymapOptions { platform, vim }
}

fn install(cx: &mut TestAppContext, platform: KeymapPlatform, vim: bool, user: &str) {
    cx.update(|cx| init_with_text(user, options(platform, vim), cx));
    let problems = cx.read(oxikube_keymap::diagnostics);
    assert!(problems.is_empty(), "{problems:?}");
}

fn press(cx: &mut TestAppContext, keys: &str, contexts: &[&str]) -> Resolution {
    cx.read(|cx| resolve(cx, keys, &parse_stack(contexts).unwrap()).unwrap())
}

fn runs(cx: &mut TestAppContext, keys: &str, contexts: &[&str]) -> Option<&'static str> {
    press(cx, keys, contexts).action()
}

/// The stack under a focused table: a cluster tab hosts a workspace of its own.
const TABLE: [&str; 5] = [
    "Workspace",
    "ClusterTab connected",
    "Workspace",
    "Pane",
    "ResourceTable kind=Pod selection=one scope=namespaced",
];
/// The same table with its filter field focused (`Editing`).
const TABLE_EDITING: [&str; 5] = [
    "Workspace",
    "ClusterTab connected",
    "Workspace",
    "Pane",
    "ResourceTable kind=Pod selection=one scope=namespaced Editing",
];
const DRAWER: [&str; 5] = [
    "Workspace",
    "ClusterTab connected",
    "Workspace",
    "Pane",
    "DetailDrawer kind=Pod",
];

/// Every other place keys are typed or mean something else: none may change with the vim layer.
const OTHER_CONTEXTS: [&[&str]; 9] = [
    &[
        "Workspace",
        "ClusterTab connected",
        "Workspace",
        "Pane",
        "Terminal",
    ],
    &[
        "Workspace",
        "ClusterTab connected",
        "Workspace",
        "Pane",
        "Terminal searching",
    ],
    &[
        "Workspace",
        "ClusterTab connected",
        "Workspace",
        "Pane",
        "ManifestEditor",
    ],
    &[
        "Workspace",
        "ClusterTab connected",
        "Palette",
        "Picker",
        "Input",
    ],
    &["Workspace", "ClusterTab connected", "JumpBar", "Input"],
    &[
        "Workspace",
        "ClusterTab connected",
        "Workspace",
        "Pane",
        "LogView",
    ],
    &[
        "Workspace",
        "ClusterTab connected",
        "Workspace",
        "Pane",
        "LogView Editing",
    ],
    &["Workspace", "Catalog", "Input"],
    &[
        "Workspace",
        "ClusterTab connected",
        "Workspace",
        "Pane",
        "ResourceTable kind=Pod Editing",
        "Input",
    ],
];

fn vim_sections() -> Vec<KeymapSection> {
    let parsed = parse_keymap(oxikube_assets::vim_keymap(), KeymapLayer::Vim).expect("parses");
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    parsed.sections.into_iter().map(|(_, s)| s).collect()
}

/// Every keystroke sequence the vim layer binds, as written.
fn vim_keys() -> BTreeSet<String> {
    vim_sections()
        .iter()
        .flat_map(|s| s.bindings.keys())
        .map(|k| k.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect()
}

#[test]
fn the_vim_file_binds_the_acceptance_keys_in_the_table_only() {
    let sections = vim_sections();
    assert_eq!(sections.len(), 1, "one declarative section");
    let section = &sections[0];
    assert_eq!(
        section.context_expr(),
        Some("ResourceTable && !Editing"),
        "vim never reaches a text field, the terminal or the editor"
    );
    let action = |keys: &str| -> String {
        match KeymapAction::from_json(&section.bindings[keys]).unwrap() {
            KeymapAction::Action { name, .. } => name,
            KeymapAction::Unbind => "null".to_owned(),
        }
    };
    for (keys, expected) in [
        ("j", "resource_table::SelectNext"),
        ("k", "resource_table::SelectPrevious"),
        ("g g", "resource_table::SelectFirst"),
        ("shift-g", "resource_table::SelectLast"),
        ("ctrl-d", "resource_table::SelectHalfPageDown"),
        ("ctrl-u", "resource_table::SelectHalfPageUp"),
        ("/", "resource_table::FocusFilter"),
        ("d d", "resource_table::DeleteSelected"),
        ("y y", "resource_table::CopyName"),
    ] {
        assert_eq!(action(keys), expected, "{keys}");
    }
}

/// Actions of `vim.json` that only move inside the table; the others must stand for a command so
/// the key, the palette and an agent run one behaviour (and `dd` stays behind the guard).
const VIM_LOCAL: &[&str] = &[
    "resource_table::SelectNext",
    "resource_table::SelectPrevious",
    "resource_table::SelectFirst",
    "resource_table::SelectLast",
    "resource_table::SelectPageDown",
    "resource_table::SelectPageUp",
    "resource_table::SelectHalfPageDown",
    "resource_table::SelectHalfPageUp",
];

#[test]
fn every_vim_binding_moves_inside_the_table_or_dispatches_a_command() {
    for section in vim_sections() {
        for value in section.bindings.values() {
            let KeymapAction::Action { name, .. } = KeymapAction::from_json(value).unwrap() else {
                continue;
            };
            let is_command = !ActionRegistry::commands_of(&name).is_empty();
            let is_local = VIM_LOCAL.contains(&name.as_str());
            assert!(
                is_command ^ is_local,
                "`{name}` must be a command (add it to `stands_for`) or table navigation (VIM_LOCAL)"
            );
        }
    }
    // Delete and copy-name are the existing guarded commands, not a new action.
    for (action, command) in [
        ("resource_table::DeleteSelected", "resource::Delete"),
        ("resource_table::CopyName", "resource::CopyName"),
    ] {
        let ids: Vec<_> = ActionRegistry::commands_of(action)
            .iter()
            .map(|id| id.as_str().to_owned())
            .collect();
        assert_eq!(ids, [command], "{action}");
    }
}

#[test]
fn vim_binds_no_typed_key_outside_the_non_editing_table() {
    // The layer's context is the only place it binds; assert the predicate never matches the
    // stacks where text is entered, for every key it binds.
    let predicate = gpui::KeyBindingContextPredicate::parse("ResourceTable && !Editing").unwrap();
    for stack in OTHER_CONTEXTS {
        let contexts = parse_stack(stack).unwrap();
        assert!(!predicate.eval(&contexts), "{stack:?}");
    }
    assert!(predicate.eval(&parse_stack(&TABLE).unwrap()));
    assert!(!predicate.eval(&parse_stack(&TABLE_EDITING).unwrap()));
}

#[gpui::test]
fn with_the_default_base_none_of_the_vim_keys_fire(cx: &mut TestAppContext) {
    for platform in [MAC, LINUX] {
        install(cx, platform, false, "");
        for keys in [
            "g g", "shift-g", "ctrl-u", "d d", "y y", "g d", "g y", "ctrl-f", "ctrl-b",
        ] {
            let found = press(cx, keys, &TABLE);
            assert_eq!(found, Resolution::Unbound, "{keys} on {platform:?}");
        }
        // The defaults keep their single keys: k9s describe, YAML and ctrl-d delete.
        assert_eq!(runs(cx, "d", &TABLE), Some("resource_table::ViewDescribe"));
        assert_eq!(runs(cx, "y", &TABLE), Some("resource_table::ViewYaml"));
        assert_eq!(
            runs(cx, "ctrl-d", &TABLE),
            Some("resource_table::DeleteSelected")
        );
    }
}

#[gpui::test]
fn with_the_vim_base_the_table_moves_like_vim(cx: &mut TestAppContext) {
    for platform in [MAC, LINUX] {
        install(cx, platform, true, "");
        for (keys, action) in [
            ("j", "resource_table::SelectNext"),
            ("k", "resource_table::SelectPrevious"),
            ("g g", "resource_table::SelectFirst"),
            ("shift-g", "resource_table::SelectLast"),
            ("ctrl-d", "resource_table::SelectHalfPageDown"),
            ("ctrl-u", "resource_table::SelectHalfPageUp"),
            ("ctrl-f", "resource_table::SelectPageDown"),
            ("ctrl-b", "resource_table::SelectPageUp"),
            ("/", "resource_table::FocusFilter"),
            ("d d", "resource_table::DeleteSelected"),
            ("y y", "resource_table::CopyName"),
            ("g d", "resource_table::ViewDescribe"),
            ("g y", "resource_table::ViewYaml"),
            // The defaults that vim does not touch.
            ("delete", "resource_table::DeleteSelected"),
            ("enter", "resource_table::OpenSelected"),
        ] {
            if keys == "enter" {
                continue; // OpenSelected is not declared in this test binary.
            }
            assert_eq!(
                runs(cx, keys, &TABLE),
                Some(action),
                "{keys} on {platform:?}"
            );
        }
        // `:` and `?` come from the defaults' `ClusterTab` section; vim needs none of its own.
        assert_eq!(runs(cx, ":", &TABLE), Some("palette::OpenJump"));
        assert_eq!(runs(cx, "?", &TABLE), Some("help::Show"));
    }
}

#[gpui::test]
fn a_lone_d_or_y_waits_for_its_pair_and_never_fires_a_verb(cx: &mut TestAppContext) {
    install(cx, MAC, true, "");
    // The first key of a pair is pending, not the default describe / YAML, so `d` then `j`
    // cannot delete (and `d` alone cannot describe a row the user did not mean to).
    assert_eq!(press(cx, "d", &TABLE), Resolution::Pending);
    assert_eq!(press(cx, "y", &TABLE), Resolution::Pending);
    assert_eq!(press(cx, "g", &TABLE), Resolution::Pending);
    // A second key that completes nothing is no binding: GPUI replays it as a fresh key press.
    assert_eq!(press(cx, "d j", &TABLE), Resolution::Unbound);
    assert_eq!(press(cx, "d y", &TABLE), Resolution::Unbound);
    assert_eq!(runs(cx, "j", &TABLE), Some("resource_table::SelectNext"));
}

#[gpui::test]
fn ctrl_d_is_half_a_page_in_vim_and_the_detail_drawer_keeps_its_keys(cx: &mut TestAppContext) {
    install(cx, MAC, true, "");
    assert_eq!(
        runs(cx, "ctrl-d", &TABLE),
        Some("resource_table::SelectHalfPageDown")
    );
    // The drawer is not a table: k9s `y` / `d` still pick its tabs, `j` / `k` still step.
    assert_eq!(runs(cx, "d", &DRAWER), Some("resource_detail::ShowTab"));
    assert_eq!(runs(cx, "y", &DRAWER), Some("resource_detail::ShowTab"));
    assert_eq!(runs(cx, "j", &DRAWER), Some("resource_detail::SelectNext"));
    assert_eq!(runs(cx, "escape", &DRAWER), Some("resource_detail::Close"));
}

#[gpui::test]
fn vim_does_not_touch_text_entry_terminal_editor_or_palette(cx: &mut TestAppContext) {
    // Typing in the table's filter, in the terminal, the editor, the palette, the jump bar and
    // every `Input`: each vim key resolves exactly as it does without the layer.
    let keys: Vec<String> = vim_keys().into_iter().collect();
    let mut plain = Vec::new();
    install(cx, MAC, false, "");
    for stack in OTHER_CONTEXTS
        .iter()
        .copied()
        .chain([TABLE_EDITING.as_slice()])
    {
        for key in &keys {
            plain.push((stack, key.clone(), press(cx, key, stack)));
        }
    }
    install(cx, MAC, true, "");
    for (stack, key, before) in plain {
        let after = press(cx, &key, stack);
        assert_eq!(
            after, before,
            "`{key}` changed in {stack:?} with the vim layer on"
        );
    }
    // The filter field is text, `ctrl-d` included, and the terminal keeps EOF.
    assert_eq!(press(cx, "ctrl-d", &OTHER_CONTEXTS[0]), Resolution::Unbound);
    for text in ["j", "k", "d", "y", "g", "/", "shift-g"] {
        assert_eq!(
            press(cx, text, &TABLE_EDITING),
            Resolution::Unbound,
            "{text}"
        );
    }
}

#[gpui::test]
fn the_user_keymap_wins_over_vim_and_null_removes_a_vim_binding(cx: &mut TestAppContext) {
    install(
        cx,
        MAC,
        true,
        r#"[{"context": "ResourceTable && !Editing",
            "bindings": {"g g": null, "d d": null, "y y": "resource_table::ViewYaml"}}]"#,
    );
    assert_eq!(
        press(cx, "g g", &TABLE),
        Resolution::Unbound,
        "null removes gg"
    );
    assert_eq!(
        press(cx, "d d", &TABLE),
        Resolution::Unbound,
        "null removes dd"
    );
    assert_eq!(
        runs(cx, "y y", &TABLE),
        Some("resource_table::ViewYaml"),
        "user rebinds yy"
    );
    assert_eq!(
        runs(cx, "shift-g", &TABLE),
        Some("resource_table::SelectLast")
    );

    // Hot reload of keymap.json keeps the vim layer.
    cx.update(|cx| reload_user_keymap(cx, "[]"));
    assert_eq!(runs(cx, "g g", &TABLE), Some("resource_table::SelectFirst"));
}

#[gpui::test]
fn base_keymap_follows_the_setting_and_hot_reloads(cx: &mut TestAppContext) {
    // The setting is registered with a default in the embedded default.json.
    cx.update(|cx| {
        cx.set_global(SettingsStore::new(oxikube_assets::default_settings()).unwrap());
        init_with_text("", options(MAC, false), cx);
    });
    let version = |cx: &mut TestAppContext| cx.read(|cx| cx.key_bindings().borrow().version());
    let setting = |cx: &mut TestAppContext| {
        cx.read(|cx| {
            use oxikube_settings::Settings as _;
            KeymapSettings::get_global(cx).base_keymap
        })
    };
    let set_user = |cx: &mut TestAppContext, text: &str| {
        cx.update(|cx| {
            SettingsStore::update_global(cx, |store, _| {
                store.set_user_settings(text).expect("valid settings");
            });
        });
        cx.run_until_parked();
    };
    assert_eq!(setting(cx), oxikube_keymap::BaseKeymap::Default);
    assert_eq!(press(cx, "g g", &TABLE), Resolution::Unbound);

    set_user(cx, r#"{ "base_keymap": "vim" }"#);
    assert_eq!(runs(cx, "g g", &TABLE), Some("resource_table::SelectFirst"));
    assert_eq!(
        runs(cx, "d d", &TABLE),
        Some("resource_table::DeleteSelected")
    );

    // An unrelated settings edit rebinds nothing.
    let bound = version(cx);
    set_user(cx, r#"{ "base_keymap": "vim", "ui_scale": 1.25 }"#);
    assert!(version(cx) == bound, "an unrelated edit must not rebind");
    assert_eq!(runs(cx, "g g", &TABLE), Some("resource_table::SelectFirst"));

    // A value that does not parse keeps the layer in force and does not crash.
    set_user(cx, r#"{ "base_keymap": "emacs" }"#);
    assert_eq!(runs(cx, "g g", &TABLE), Some("resource_table::SelectFirst"));

    set_user(cx, r#"{ "base_keymap": "default" }"#);
    assert_eq!(
        press(cx, "g g", &TABLE),
        Resolution::Unbound,
        "switched back at runtime"
    );
    assert_eq!(runs(cx, "d", &TABLE), Some("resource_table::ViewDescribe"));
}

#[gpui::test]
fn a_vim_base_keymap_in_settings_is_in_force_from_the_first_merge(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let mut store = SettingsStore::new(oxikube_assets::default_settings()).unwrap();
        store
            .set_user_settings(r#"{ "base_keymap": "vim" }"#)
            .unwrap();
        cx.set_global(store);
        // Built with the flag off: the setting decides.
        init_with_text("", options(MAC, false), cx);
    });
    assert_eq!(runs(cx, "g g", &TABLE), Some("resource_table::SelectFirst"));
}

#[test]
fn the_setting_has_a_default_and_a_schema_entry() {
    let defaults: serde_json::Value =
        serde_json_lenient::from_str(oxikube_assets::default_settings()).expect("default.json");
    assert_eq!(defaults["base_keymap"], "default");

    let schema: serde_json::Value = serde_json::from_str(include_str!(
        "../../oxikube_assets/assets/settings/settings.schema.json"
    ))
    .expect("schema");
    assert!(schema["properties"]["base_keymap"].is_object());
    let names: Vec<&str> = schema["$defs"]["BaseKeymap"]["oneOf"]
        .as_array()
        .expect("an enum")
        .iter()
        .map(|v| v["const"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["default", "vim"]);
}
