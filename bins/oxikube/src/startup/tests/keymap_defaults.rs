//! The shipped default keymaps against the real init order (E11-S07): every action they name is
//! registered by a crate of the app or is one of the few whose feature has not landed, the
//! keymap loads without a diagnostic, and the k9s verbs resolve in the contexts the views set.

use std::collections::BTreeSet;

use gpui::TestAppContext;
use oxikube_keymap::{ActionRegistry, KeymapAction, KeymapLayer, Resolution, parse_stack, resolve};

use crate::startup::{StartupEnv, init};

/// Actions the default keymaps name that no crate of this build registers yet. Their bindings are
/// skipped silently until the owning story declares the action: the `:` jump bar (E11-S05) and
/// the help overlay (E11-S10). Remove an entry when its story lands; the
/// test fails the other way too, so the list cannot go stale.
const NOT_YET_REGISTERED: [&str; 2] = ["help::Show", "palette::OpenJump"];

fn named_actions(text: &str) -> BTreeSet<String> {
    let parsed = oxikube_keymap::file::parse_keymap(text, KeymapLayer::Default).unwrap();
    let mut names = BTreeSet::new();
    for (_, section) in parsed.sections {
        for value in section.bindings.values() {
            if let Ok(KeymapAction::Action { name, .. }) = KeymapAction::from_json(&value.clone()) {
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
