//! The shipped default keymaps against the real init order (E11-S07): every action they name is
//! registered by a crate of the app or is one of the few whose feature has not landed, the
//! keymap loads without a diagnostic, and the k9s verbs resolve in the contexts the views set.

use std::collections::BTreeSet;

use gpui::TestAppContext;
use oxikube_keymap::{ActionRegistry, KeymapAction, KeymapLayer, Resolution, parse_stack, resolve};

use crate::startup::{StartupEnv, init};

/// Actions the default keymaps name that no crate of this build registers yet. Their bindings are
/// skipped silently until the owning story declares the action: the help overlay (E11-S10).
/// Remove an entry when its story lands; the test fails the other way too, so the list cannot go
/// stale.
const NOT_YET_REGISTERED: [&str; 1] = ["help::Show"];

fn named_actions(text: &str) -> BTreeSet<String> {
    let parsed = oxikube_keymap::file::parse_keymap(text, KeymapLayer::Default).unwrap();
    let mut names = BTreeSet::new();
    for (_, section) in parsed.sections {
        for value in section.bindings.values() {
            if let Ok(KeymapAction::Action { name, .. }) = KeymapAction::from_json(value) {
                names.insert(name);
            }
        }
    }
    names
}

#[gpui::test]
fn the_apps_crates_register_every_action_the_default_keymaps_name(cx: &mut TestAppContext) {
    cx.update(|cx| init(cx, StartupEnv::test())).expect("init");
    let registry = cx.read(ActionRegistry::from_app);
    for platform in [
        oxikube_keymap::KeymapPlatform::MacOs,
        oxikube_keymap::KeymapPlatform::Linux,
        oxikube_keymap::KeymapPlatform::Windows,
    ] {
        let missing: BTreeSet<String> = named_actions(oxikube_assets::default_keymap(platform))
            .into_iter()
            .filter(|name| !registry.contains(name))
            .collect();
        let expected: BTreeSet<String> =
            NOT_YET_REGISTERED.iter().map(|s| (*s).to_owned()).collect();
        assert_eq!(
            missing, expected,
            "{platform:?}: a default binds an action nobody registers (a typo, or a crate that is \
             not initialised), or NOT_YET_REGISTERED is stale"
        );
    }
}

#[gpui::test]
fn the_default_keymap_loads_clean_and_binds_the_verbs(cx: &mut TestAppContext) {
    cx.update(|cx| init(cx, StartupEnv::test())).expect("init");
    let problems = cx.read(oxikube_keymap::diagnostics);
    assert!(problems.is_empty(), "{problems:?}");

    let table = parse_stack(&[
        "Workspace",
        "ClusterTab connected",
        "Workspace",
        "Pane",
        "ResourceTable kind=Pod selection=one",
    ])
    .unwrap();
    for (keys, action) in [
        ("y", "resource_table::ViewYaml"),
        ("d", "resource_table::ViewDescribe"),
        ("e", "resource_table::EditSelected"),
        ("ctrl-d", "resource_table::DeleteSelected"),
        ("l", "resource_table::ViewLogs"),
        ("s", "resource_table::ShellSelected"),
        ("shift-f", "resource_table::PortForward"),
        ("f", "resource_table::ShowPortForwards"),
        ("ctrl-w", "resource_table::ToggleWide"),
        ("/", "resource_table::FocusFilter"),
    ] {
        let resolution = cx.read(|cx| resolve(cx, keys, &table).unwrap());
        assert_eq!(resolution.action(), Some(action), "{keys}");
    }

    // The terminal keeps ctrl-d (EOF) in the real keymap, workspace chords included.
    let terminal = parse_stack(&[
        "Workspace",
        "ClusterTab connected",
        "Workspace",
        "Pane",
        "Terminal",
    ])
    .unwrap();
    for keys in ["ctrl-d", "ctrl-w", "y", "l", ":", "?"] {
        let resolution = cx.read(|cx| resolve(cx, keys, &terminal).unwrap());
        assert_eq!(resolution, Resolution::Unbound, "{keys} in a terminal");
    }
}

/// The vim base keymap (E11-S09): every action `vim.json` names is registered by a crate of the
/// app, so none of its bindings is silently skipped.
#[gpui::test]
fn the_apps_crates_register_every_action_the_vim_keymap_names(cx: &mut TestAppContext) {
    cx.update(|cx| init(cx, StartupEnv::test())).expect("init");
    let registry = cx.read(ActionRegistry::from_app);
    let parsed =
        oxikube_keymap::file::parse_keymap(oxikube_assets::vim_keymap(), KeymapLayer::Vim).unwrap();
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let mut checked = 0;
    for (_, section) in parsed.sections {
        for value in section.bindings.values() {
            if let Ok(KeymapAction::Action { name, .. }) = KeymapAction::from_json(&value.clone()) {
                assert!(
                    registry.contains(&name),
                    "vim.json names `{name}`; no crate registers it"
                );
                checked += 1;
            }
        }
    }
    assert!(checked >= 10, "vim.json binds {checked} actions");
}

/// Starts the app over a config dir whose `settings.json` is `settings` and checks the vim keys
/// of the story in a table (on exactly when `vim`) and in a terminal (never).
fn check_base_keymap(cx: &mut TestAppContext, settings: &str, vim: bool) {
    let stack = |last: &str| {
        parse_stack(&[
            "Workspace",
            "ClusterTab connected",
            "Workspace",
            "Pane",
            last,
        ])
        .unwrap()
    };
    let (table, terminal) = (
        stack("ResourceTable kind=Pod selection=one"),
        stack("Terminal"),
    );
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("settings.json"), settings).unwrap();
    cx.update(|cx| {
        let mut env = StartupEnv::test();
        env.config = crate::startup::ConfigSource::Dir(dir.path().to_owned());
        init(cx, env).unwrap();
    });
    let problems = cx.read(oxikube_keymap::diagnostics);
    assert!(problems.is_empty(), "{problems:?}");
    for (keys, action) in [
        ("g g", "resource_table::SelectFirst"),
        ("shift-g", "resource_table::SelectLast"),
        ("ctrl-d", "resource_table::SelectHalfPageDown"),
        ("ctrl-u", "resource_table::SelectHalfPageUp"),
        ("d d", "resource_table::DeleteSelected"),
        ("y y", "resource_table::CopyName"),
    ] {
        let found = cx.read(|cx| resolve(cx, keys, &table).unwrap());
        assert_eq!(found.action() == Some(action), vim, "{keys}: {settings}");
        // The terminal is never touched: ctrl-d is EOF there, the rest is typing.
        let in_terminal = cx.read(|cx| resolve(cx, keys, &terminal).unwrap());
        assert_eq!(in_terminal, Resolution::Unbound, "{keys} in a terminal");
    }
}

#[gpui::test]
fn the_vim_layer_is_off_unless_settings_json_asks_for_it(cx: &mut TestAppContext) {
    check_base_keymap(cx, "{}", false);
}

#[gpui::test]
fn base_keymap_default_in_settings_json_is_the_plain_defaults(cx: &mut TestAppContext) {
    check_base_keymap(cx, r#"{ "base_keymap": "default" }"#, false);
}

#[gpui::test]
fn base_keymap_vim_in_settings_json_turns_the_vim_layer_on(cx: &mut TestAppContext) {
    check_base_keymap(cx, "// vim, please\n{ \"base_keymap\": \"vim\" }", true);
}
