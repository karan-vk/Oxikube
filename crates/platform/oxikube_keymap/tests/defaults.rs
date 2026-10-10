//! The shipped default keymaps as data (E11-S07): no ambiguous key, no binding that dispatches
//! nothing, every Phase 1 context has a home, and a snapshot of the bindings per context so a
//! change to a default shows in review.
//!
//! These read the embedded JSON only (no action needs to be registered), so they run in every
//! build. `tests/k9s.rs` resolves keys against the installed keymap; the binary's
//! `default_keymaps` test checks the real crates register what the files name.
//!
//! Update the snapshots after an intended change with
//! `OXIKUBE_UPDATE_SNAPSHOTS=1 cargo test -p oxikube_keymap --test defaults`.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::PathBuf;

use oxikube_keymap::file::{KeymapSection, parse_keymap};
use oxikube_keymap::{ActionRegistry, KeymapAction, KeymapLayer, KeymapPlatform, contexts};

const PLATFORMS: [(&str, KeymapPlatform); 3] = [
    ("macos", KeymapPlatform::MacOs),
    ("linux", KeymapPlatform::Linux),
    ("windows", KeymapPlatform::Windows),
];

fn sections(text: &str, layer: KeymapLayer) -> Vec<KeymapSection> {
    let parsed = parse_keymap(text, layer).expect("embedded keymap parses");
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    parsed.sections.into_iter().map(|(_, s)| s).collect()
}

fn defaults(platform: KeymapPlatform) -> Vec<KeymapSection> {
    sections(
        oxikube_assets::default_keymap(platform),
        KeymapLayer::Default,
    )
}

/// A context expression with its whitespace normalised, `None` for "everywhere".
fn context_of(section: &KeymapSection) -> String {
    section.context_expr().map_or_else(
        || "(everywhere)".to_owned(),
        |c| c.split_whitespace().collect::<Vec<_>>().join(" "),
    )
}

fn action_text(value: &serde_json::Value) -> String {
    match KeymapAction::from_json(value).expect("a valid binding value") {
        KeymapAction::Unbind => "null".to_owned(),
        KeymapAction::Action { name, data: None } => name,
        KeymapAction::Action {
            name,
            data: Some(data),
        } => format!("{name} {data}"),
    }
}

#[test]
fn no_key_is_bound_twice_in_one_context() {
    // Two sections with the same context and a binding for the same keystrokes are ambiguous at
    // equal specificity: the later one silently wins. Distinct contexts are the way to say
    // "this key means something else there".
    for (name, platform) in PLATFORMS {
        let mut seen: BTreeMap<(String, String), String> = BTreeMap::new();
        for section in defaults(platform) {
            let context = context_of(&section);
            for (keystrokes, value) in &section.bindings {
                let keys = keystrokes.split_whitespace().collect::<Vec<_>>().join(" ");
                let action = action_text(value);
                if let Some(first) = seen.insert((context.clone(), keys.clone()), action.clone()) {
                    panic!(
                        "{name}: `{keys}` in `{context}` is bound to `{first}` and again to `{action}`"
                    );
                }
            }
        }
    }
}

/// Actions of the default keymaps that move a cursor, a selection or the focus inside one view,
/// or are OS window chrome. They are view business, not commands: they have no MCP tool and
/// no palette entry. Anything else a default binds must dispatch a command (be one, or stand
/// for one in `oxikube_keymap::stands_for`).
const VIEW_LOCAL: &[&str] = &[
    // The table's cursor and selection, and its filter field.
    "resource_table::SelectNext",
    "resource_table::SelectPrevious",
    "resource_table::SelectFirst",
    "resource_table::SelectLast",
    "resource_table::SelectPageDown",
    "resource_table::SelectPageUp",
    "resource_table::ExtendNext",
    "resource_table::ExtendPrevious",
    "resource_table::ClearSelection",
    "resource_table::ClearFilter",
    // The detail drawer: close, step to the next object, switch tab.
    "resource_detail::Close",
    "resource_detail::SelectNext",
    "resource_detail::SelectPrevious",
    "resource_detail::ShowTab",
    // The catalog home's list.
    "catalog::SelectNext",
    "catalog::SelectPrevious",
    "catalog::SelectFirst",
    "catalog::SelectLast",
    "catalog::FocusSearch",
    // A picker's selection, confirm and cancel (E11-S02): the palette, the container chooser.
    "picker::SelectNext",
    "picker::SelectPrevious",
    "picker::SelectFirst",
    "picker::SelectLast",
    "picker::Confirm",
    "picker::SecondaryConfirm",
    "picker::Cancel",
    // The `:` jump bar's Tab (E11-S05): replaces the word being typed by the selected completion.
    "jump_bar::Complete",
    // The log viewer's selection.
    "log_view::ClearSelection",
    // The bottom dock's terminal panel and the OS application menu.
    "terminal_panel::TogglePanel",
    "oxikube::Hide",
    "oxikube::HideOthers",
    "oxikube::Minimize",
    "oxikube::OpenPreferences",
];

/// Actions bound by a default whose handler only toasts "not available yet" (port forwarding has
/// not landed): they stand for no command and are not view navigation either. An entry leaves this
/// list when its handler dispatches a command and `stands_for` names it.
const PLACEHOLDERS: &[&str] = &["resource_table::ShowPortForwards"];

fn action_names(platform: KeymapPlatform) -> Vec<String> {
    let mut names = Vec::new();
    for section in defaults(platform) {
        for value in section.bindings.values() {
            if let KeymapAction::Action { name, .. } = KeymapAction::from_json(value).unwrap() {
                if !names.contains(&name) {
                    names.push(name);
                }
            }
        }
    }
    names
}

#[test]
fn every_default_binding_dispatches_a_command_or_moves_inside_a_view() {
    for (os, platform) in PLATFORMS {
        for name in action_names(platform) {
            let is_command = !ActionRegistry::commands_of(&name).is_empty();
            let is_local = VIEW_LOCAL.contains(&name.as_str());
            if PLACEHOLDERS.contains(&name.as_str()) {
                assert!(
                    !is_command && !is_local,
                    "{os}: `{name}` now stands for a command: remove it from PLACEHOLDERS"
                );
                continue;
            }
            assert!(
                is_command || is_local,
                "{os}: `{name}` dispatches no command: declare the command and add the action to \
                 `stands_for`, or list it in VIEW_LOCAL if it only moves inside a view"
            );
            assert!(
                !(is_command && is_local),
                "{os}: `{name}` is a command: remove it from VIEW_LOCAL"
            );
        }
    }
}

#[test]
fn the_view_local_list_names_only_actions_a_default_binds() {
    let bound: Vec<String> = PLATFORMS
        .iter()
        .flat_map(|(_, platform)| action_names(*platform))
        .collect();
    for name in VIEW_LOCAL {
        assert!(
            bound.iter().any(|b| b == name),
            "`{name}` is not bound by any default"
        );
    }
}

#[test]
fn the_phase_1_contexts_are_the_nine_of_the_story() {
    assert_eq!(
        contexts::PHASE_1,
        [
            "Workspace",
            "ClusterTab",
            "ResourceTable",
            "DetailDrawer",
            "LogView",
            "Terminal",
            "ManifestEditor",
            "Palette",
            "JumpBar",
        ]
    );
}

/// Whether `expr` names `context` as an identifier (`ResourceTable && !Editing` names
/// `ResourceTable`, `Terminal > Input` names both).
fn names(expr: &str, context: &str) -> bool {
    expr.split(|c: char| !c.is_alphanumeric() && c != '_')
        .any(|word| word == context)
}

#[test]
fn every_phase_1_context_has_a_section_in_every_os_default() {
    for (os, platform) in PLATFORMS {
        let exprs: Vec<String> = defaults(platform)
            .iter()
            .filter_map(|s| s.context_expr().map(str::to_owned))
            .collect();
        for context in contexts::PHASE_1 {
            // The workspace's own chords (splits, docks, close tab) are still bound in Rust by
            // `oxikube_workspace::actions` and its window menus; the root sets the context.
            if context == contexts::WORKSPACE {
                continue;
            }
            assert!(
                exprs.iter().any(|expr| names(expr, context)),
                "{os}: no default section names the `{context}` context"
            );
        }
    }
}

/// Whether `keystrokes` is a single printable character with no chord modifier: typing it.
fn is_typing(keystrokes: &str) -> bool {
    let mut parts = keystrokes.split_whitespace();
    let (Some(first), None) = (parts.next(), parts.next()) else {
        return false;
    };
    let stroke = gpui::Keystroke::parse(first).expect("a valid keystroke");
    let m = stroke.modifiers;
    stroke.key.chars().count() == 1 && !(m.control || m.alt || m.platform || m.function)
}

#[test]
fn views_that_take_text_bind_no_typed_character() {
    // Terminal, editor, palette and jump bar field: a typed character is text there. Only a
    // `null` (which gives the key back to the field) may name them for a typing key.
    let text_contexts = [
        contexts::TERMINAL,
        contexts::MANIFEST_EDITOR,
        contexts::PALETTE,
        contexts::JUMP_BAR,
    ];
    for (os, platform) in PLATFORMS {
        for section in defaults(platform) {
            let Some(expr) = section.context_expr() else {
                continue;
            };
            if !text_contexts.iter().any(|text| names(expr, text)) {
                continue;
            }
            for (keystrokes, value) in &section.bindings {
                let is_null = matches!(KeymapAction::from_json(value), Ok(KeymapAction::Unbind));
                assert!(
                    is_null || !is_typing(keystrokes),
                    "{os}: `{keystrokes}` is bound in the text context `{expr}`"
                );
            }
        }
    }
}

#[test]
fn typed_letters_are_bound_only_while_no_text_field_has_the_focus() {
    // Tables, logs and the catalog flag `Editing` while their field has the focus. A section of
    // theirs that binds a typed character must exclude it (`!Editing`), unless the section is
    // the field's own (`Editing`), where it is the field's escape hatch and binds none.
    let guarded = [contexts::RESOURCE_TABLE, contexts::LOGS, contexts::CATALOG];
    for (os, platform) in PLATFORMS {
        for section in defaults(platform) {
            let Some(expr) = section.context_expr() else {
                continue;
            };
            if !guarded.iter().any(|c| names(expr, c)) {
                continue;
            }
            for (keystrokes, value) in &section.bindings {
                if matches!(KeymapAction::from_json(value), Ok(KeymapAction::Unbind)) {
                    continue;
                }
                if is_typing(keystrokes) {
                    assert!(
                        expr.contains("!Editing") || expr.contains("!Input"),
                        "{os}: typing key `{keystrokes}` in `{expr}` would fire while typing: \
                         add `!Editing`"
                    );
                }
            }
        }
    }
}

#[test]
fn windows_defaults_follow_linux() {
    // Windows is not interactive-tested yet; it takes the Linux chords. Adding
    // `default-windows.json` for real is a data change (the loader is OS-keyed).
    let body = |text: &str| text.lines().skip(1).collect::<Vec<_>>().join("\n");
    assert_eq!(
        body(oxikube_assets::default_keymap(KeymapPlatform::Linux)),
        body(oxikube_assets::default_keymap(KeymapPlatform::Windows))
    );
}

fn render(platform: KeymapPlatform) -> String {
    let mut out = String::new();
    for section in defaults(platform) {
        let _ = writeln!(out, "[{}]", context_of(&section));
        let mut rows: Vec<(String, String)> = section
            .bindings
            .iter()
            .map(|(keys, value)| {
                (
                    keys.split_whitespace().collect::<Vec<_>>().join(" "),
                    action_text(value),
                )
            })
            .collect();
        rows.sort();
        for (keys, action) in rows {
            let _ = writeln!(out, "  {keys:<16} {action}");
        }
    }
    out
}

fn snapshot_path(os: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/snapshots")
        .join(format!("default-{os}.txt"))
}

#[test]
fn the_bindings_per_context_match_the_snapshots() {
    let update = std::env::var_os("OXIKUBE_UPDATE_SNAPSHOTS").is_some();
    for (os, platform) in [PLATFORMS[0], PLATFORMS[1]] {
        let actual = render(platform);
        let path = snapshot_path(os);
        if update {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, &actual).unwrap();
            continue;
        }
        let expected = std::fs::read_to_string(&path).unwrap_or_else(|err| {
            panic!(
                "{}: {err}; run with OXIKUBE_UPDATE_SNAPSHOTS=1",
                path.display()
            )
        });
        assert!(
            expected == actual,
            "the {os} default bindings changed; review the diff and rerun with \
             OXIKUBE_UPDATE_SNAPSHOTS=1\n--- {}\n{}",
            path.display(),
            diff(&expected, &actual)
        );
    }
}

/// The lines that differ, `-` from the snapshot and `+` from the shipped file.
fn diff(expected: &str, actual: &str) -> String {
    let old: Vec<&str> = expected.lines().collect();
    let new: Vec<&str> = actual.lines().collect();
    let mut out = String::new();
    for line in &old {
        if !new.contains(line) {
            let _ = writeln!(out, "- {line}");
        }
    }
    for line in &new {
        if !old.contains(line) {
            let _ = writeln!(out, "+ {line}");
        }
    }
    out
}
