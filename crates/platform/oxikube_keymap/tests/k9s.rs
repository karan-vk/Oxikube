//! The k9s verbs of the default keymaps, resolved per context (E11-S07).
//!
//! The shipped `default-macos.json` and `default-linux.json` are installed for real; the actions
//! they name are declared here under their real names (the crates that own them are not linked
//! into this test), so a binding is live exactly as in the app. Each test then asks what a key
//! does for a context stack written as data (`oxikube_keymap::resolve`): the same question GPUI
//! answers when a key is pressed, without a window. The `#[gpui::test]`s of the owning crates
//! (`oxikube_resources_ui`, `oxikube_terminal`) press the keys in real views.

use gpui::{Action, TestAppContext, actions};
use oxikube_domain::command::CommandId;
use oxikube_keymap::{
    KeymapOptions, KeymapPlatform, Resolution, active_bindings, bindings_for_command,
    init_with_text, parse_stack, resolve,
};
use schemars::JsonSchema;
use serde::Deserialize;

actions!(
    resource_table,
    [
        ViewYaml,
        ViewDescribe,
        EditSelected,
        ViewLogs,
        PortForward,
        ShowPortForwards,
        ToggleWide,
        DeleteSelected,
        ShellSelected,
        AttachSelected,
        FocusFilter,
        ClearFilter,
        ClearSelection
    ]
);
actions!(resource_detail, [Close]);
actions!(palette, [Toggle, OpenJump]);
actions!(help, [Show]);
actions!(terminal, [Copy, Split]);
actions!(log_view, [ToggleAutoscroll, ToggleWrap, Find]);

#[derive(Clone, PartialEq, Deserialize, JsonSchema, Action)]
#[action(namespace = resource_detail)]
struct ShowTab {
    index: u8,
}

const MAC: KeymapPlatform = KeymapPlatform::MacOs;

/// How GPUI prints the macOS `cmd` modifier: `cmd` on macOS, `super` on the other hosts, whatever
/// platform the keymap under test is for.
const CMD: &str = if cfg!(target_os = "macos") {
    "cmd"
} else {
    "super"
};
const LINUX: KeymapPlatform = KeymapPlatform::Linux;

fn install(cx: &mut TestAppContext, platform: KeymapPlatform, user: &str) {
    cx.update(|cx| {
        init_with_text(
            user,
            KeymapOptions {
                platform,
                vim: false,
            },
            cx,
        )
    });
    let problems = cx.read(oxikube_keymap::diagnostics);
    assert!(problems.is_empty(), "{problems:?}");
}

/// What `keys` does in the context stack `contexts`, outermost first.
fn press(cx: &mut TestAppContext, keys: &str, contexts: &[&str]) -> Resolution {
    cx.read(|cx| resolve(cx, keys, &parse_stack(contexts).unwrap()).unwrap())
}

/// The action `keys` runs in `contexts`, `None` when it runs none.
fn runs(cx: &mut TestAppContext, keys: &str, contexts: &[&str]) -> Option<&'static str> {
    press(cx, keys, contexts).action()
}

/// The stack under a focused table of `kind` (a cluster tab hosts a workspace of its own).
fn table(kind: &str, extra: &str) -> Vec<String> {
    vec![
        "Workspace".into(),
        "ClusterTab connected".into(),
        "Workspace".into(),
        "Pane".into(),
        format!("ResourceTable kind={kind} selection=one scope=namespaced {extra}"),
    ]
}

fn strs(parts: &[String]) -> Vec<&str> {
    parts.iter().map(String::as_str).collect()
}

const TERMINAL: [&str; 5] = [
    "Workspace",
    "ClusterTab connected",
    "Workspace",
    "Pane",
    "Terminal",
];

/// The verbs of the story, with the view action each is bound to in a table.
const VERBS: [(&str, &str); 10] = [
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
];

#[gpui::test]
fn the_k9s_verbs_run_in_a_focused_table_on_both_oses(cx: &mut TestAppContext) {
    for platform in [MAC, LINUX] {
        install(cx, platform, "");
        let stack = table("Pod", "");
        for (keys, action) in VERBS {
            assert_eq!(
                runs(cx, keys, &strs(&stack)),
                Some(action),
                "{keys} on {platform:?}"
            );
        }
    }
}

/// Verbs whose handler only toasts "not available yet" (port forwarding has not landed), so they
/// stand for no command.
const PLACEHOLDER_VERBS: &[&str] = &["resource_table::ShowPortForwards"];

#[gpui::test]
fn every_verb_is_a_command_or_stands_for_one(cx: &mut TestAppContext) {
    use oxikube_keymap::ActionRegistry;
    install(cx, MAC, "");
    for (keys, action) in VERBS {
        let commands = ActionRegistry::commands_of(action);
        if PLACEHOLDER_VERBS.contains(&action) {
            // A placeholder must not claim a command it does not dispatch; once it does, it
            // leaves this list.
            assert!(
                commands.is_empty(),
                "{keys} -> {action} now stands for a command: drop it from PLACEHOLDER_VERBS"
            );
        } else {
            assert!(
                !commands.is_empty(),
                "{keys} -> {action} dispatches no command"
            );
        }
    }
}

#[gpui::test]
fn a_letter_typed_into_the_filter_is_text_on_both_oses(cx: &mut TestAppContext) {
    for platform in [MAC, LINUX] {
        install(cx, platform, "");
        // The table's key context says `Editing` while its filter field has the focus; the
        // field's own context sits below it.
        let editing = table("Pod", "Editing");
        let mut field = editing.clone();
        field.push("Input".into());
        for (keys, action) in VERBS {
            for stack in [&editing, &field] {
                assert_eq!(
                    press(cx, keys, &strs(stack)),
                    Resolution::Unbound,
                    "{keys} ({action}) while typing on {platform:?}"
                );
            }
        }
        // Escape clears the filter and returns to the rows.
        assert_eq!(
            runs(cx, "escape", &strs(&editing)),
            Some("resource_table::ClearFilter")
        );
    }
}

#[gpui::test]
fn a_terminal_keeps_every_key_a_shell_uses(cx: &mut TestAppContext) {
    for platform in [MAC, LINUX] {
        install(cx, platform, "");
        // ctrl-d is EOF, ctrl-w deletes a word; the letters and `/` are typed.
        for keys in [
            "ctrl-d", "ctrl-w", "y", "d", "e", "l", "s", "f", "shift-f", "/",
        ] {
            assert_eq!(
                press(cx, keys, &TERMINAL),
                Resolution::Unbound,
                "{keys} must reach the shell on {platform:?}"
            );
        }
    }
}

#[gpui::test]
fn the_prompts_open_from_a_cluster_tab_and_are_characters_where_text_is_typed(
    cx: &mut TestAppContext,
) {
    for platform in [MAC, LINUX] {
        install(cx, platform, "");
        let tab = table("Pod", "");
        let overview = ["Workspace", "ClusterTab connected", "Workspace", "Pane"];
        for stack in [strs(&tab), overview.to_vec()] {
            // The keypress of a US keyboard: `;` with shift typed `:`. GPUI matches the typed
            // character, so the binding is written `:`; a test names the pair as `key->char`.
            assert_eq!(
                runs(cx, "shift-;->:", &stack),
                Some("palette::OpenJump"),
                "{stack:?}"
            );
            assert_eq!(
                runs(cx, "shift-/->?", &stack),
                Some("help::Show"),
                "{stack:?}"
            );
            assert_eq!(runs(cx, ":", &stack), Some("palette::OpenJump"));
            assert_eq!(runs(cx, "?", &stack), Some("help::Show"));
        }
        // The catalog home is not in a cluster tab: no prompts there.
        assert_eq!(
            press(cx, ":", &["Workspace", "Catalog"]),
            Resolution::Unbound
        );

        // Text is typed in the shell, the editor, the palette and jump bar fields, and any input.
        let editor = [
            "Workspace",
            "ClusterTab",
            "Workspace",
            "Pane",
            "ManifestEditor",
        ];
        let palette = ["Workspace", "Palette", "Input"];
        let jump = ["Workspace", "ClusterTab", "JumpBar", "Input"];
        let filter = table("Pod", "Editing");
        let mut input = filter.clone();
        input.push("Input".into());
        for stack in [
            TERMINAL.to_vec(),
            editor.to_vec(),
            palette.to_vec(),
            jump.to_vec(),
            strs(&input),
        ] {
            for keys in [":", "?", "shift-;->:", "shift-/->?"] {
                assert_eq!(
                    press(cx, keys, &stack),
                    Resolution::Unbound,
                    "{keys} must be a character in {stack:?} on {platform:?}"
                );
            }
        }
    }
}

#[gpui::test]
fn the_detail_drawer_switches_tabs_with_y_and_d(cx: &mut TestAppContext) {
    for platform in [MAC, LINUX] {
        install(cx, platform, "");
        let drawer = [
            "Workspace",
            "ClusterTab connected",
            "Workspace",
            "Dock position=right",
            "DetailDrawer mount=drawer kind=Pod",
        ];
        for (keys, tab) in [("y", 2u8), ("d", 3), ("1", 1)] {
            match press(cx, keys, &drawer) {
                Resolution::Action { name, binding } => {
                    assert_eq!(name, "resource_detail::ShowTab", "{keys}");
                    assert_eq!(binding.keystrokes, [keys]);
                    let shown = cx.read(|cx| {
                        let keymap = cx.key_bindings();
                        let keymap = keymap.borrow();
                        keymap
                            .bindings()
                            .filter(|b| b.action().name() == name)
                            .find(|b| b.keystrokes()[0].inner().unparse() == keys)
                            .and_then(|b| {
                                b.action()
                                    .as_any()
                                    .downcast_ref::<ShowTab>()
                                    .map(|a| a.index)
                            })
                    });
                    assert_eq!(shown, Some(tab), "{keys} on {platform:?}");
                }
                other => panic!("{keys}: {other:?}"),
            }
        }
        assert_eq!(runs(cx, "escape", &drawer), Some("resource_detail::Close"));
        // The drawer takes no table verb it cannot answer for.
        assert_eq!(runs(cx, "ctrl-w", &drawer), None);
    }
}

#[gpui::test]
fn a_log_view_keeps_its_own_letters(cx: &mut TestAppContext) {
    for platform in [MAC, LINUX] {
        install(cx, platform, "");
        let logs = [
            "Workspace",
            "ClusterTab connected",
            "Workspace",
            "Pane",
            "LogView",
        ];
        assert_eq!(runs(cx, "s", &logs), Some("log_view::ToggleAutoscroll"));
        assert_eq!(runs(cx, "w", &logs), Some("log_view::ToggleWrap"));
        // The table's verbs are not the log view's.
        assert_eq!(runs(cx, "ctrl-d", &logs), None);
        assert_eq!(runs(cx, "y", &logs), None);
        let searching = [
            "Workspace",
            "ClusterTab",
            "Pane",
            "LogView Editing searching",
        ];
        assert_eq!(press(cx, "s", &searching), Resolution::Unbound);
    }
}

#[gpui::test]
fn a_section_can_scope_a_binding_to_a_kind(cx: &mut TestAppContext) {
    // E12 binds `s` to scale on Deployments this way; the table's context carries `kind`.
    install(
        cx,
        MAC,
        r#"[{"context": "ResourceTable && !Editing && kind == Deployment",
             "bindings": {"s": "resource_table::ToggleWide"}}]"#,
    );
    let deployments = table("Deployment", "");
    let pods = table("Pod", "");
    assert_eq!(
        runs(cx, "s", &strs(&deployments)),
        Some("resource_table::ToggleWide")
    );
    assert_eq!(
        runs(cx, "s", &strs(&pods)),
        Some("resource_table::ShellSelected")
    );
    // The default for every other kind still applies to the deployment's other keys.
    assert_eq!(
        runs(cx, "y", &strs(&deployments)),
        Some("resource_table::ViewYaml")
    );
}

#[gpui::test]
fn a_user_binding_overrides_a_verb_and_null_removes_it(cx: &mut TestAppContext) {
    install(
        cx,
        LINUX,
        r#"[{"context": "ResourceTable", "bindings": {"y": "resource_table::ViewLogs", "d": null}}]"#,
    );
    let stack = table("Pod", "");
    assert_eq!(
        runs(cx, "y", &strs(&stack)),
        Some("resource_table::ViewLogs")
    );
    assert_eq!(press(cx, "d", &strs(&stack)), Resolution::Unbound);
}

#[gpui::test]
fn the_palette_finds_the_keys_of_a_command_through_its_view_actions(cx: &mut TestAppContext) {
    install(cx, MAC, "");
    let keys = |cx: &mut TestAppContext, command| {
        cx.read(|cx| {
            bindings_for_command(cx, command)
                .into_iter()
                .map(|info| info.keystrokes_text())
                .collect::<Vec<_>>()
        })
    };
    assert_eq!(keys(cx, CommandId::RESOURCE_VIEW_YAML), ["y"]);
    assert_eq!(keys(cx, CommandId::RESOURCE_VIEW_DESCRIBE), ["d"]);
    assert_eq!(keys(cx, CommandId::TABLE_TOGGLE_WIDE), ["ctrl-w"]);
    // Both resource and log shells are one key each.
    assert_eq!(keys(cx, CommandId::POD_SHELL), ["s"]);
    // A command named by its own action: the jump bar's `:` and the palette's chord.
    assert_eq!(keys(cx, CommandId::PALETTE_OPEN_JUMP), [":"]);
    assert_eq!(
        keys(cx, CommandId::PALETTE_TOGGLE),
        [format!("{CMD}-shift-p")]
    );
    // `?` opens the help overlay from a cluster tab and, in the overlay's empty search field
    // (E11-S10), closes it again: two bindings, one key.
    assert_eq!(keys(cx, CommandId::HELP_SHOW), ["?", "?"]);
    // No key for a command nobody bound.
    assert!(keys(cx, CommandId::POD_EXEC).is_empty());
}

#[gpui::test]
fn the_bindings_in_force_follow_the_context_stack(cx: &mut TestAppContext) {
    install(cx, MAC, "");
    let in_force = |cx: &mut TestAppContext, contexts: &[&str]| {
        cx.read(|cx| {
            active_bindings(cx, &parse_stack(contexts).unwrap())
                .into_iter()
                .map(|b| (b.action, b.binding.keystrokes_text()))
                .collect::<Vec<_>>()
        })
    };
    let table_stack = table("Pod", "");
    let in_table = in_force(cx, &strs(&table_stack));
    for (keys, action) in VERBS {
        assert!(
            in_table.contains(&(action, keys.to_owned())),
            "{keys} -> {action} is in force in a table"
        );
    }
    assert!(in_table.contains(&("palette::OpenJump", ":".to_owned())));
    assert!(
        !in_table
            .iter()
            .any(|(action, _)| action.starts_with("terminal::")),
        "terminal keys are not in force in a table: {in_table:?}"
    );
    let editing = table("Pod", "Editing");
    let while_typing = in_force(cx, &strs(&editing));
    assert!(
        !while_typing
            .iter()
            .any(|(_, keys)| keys == "y" || keys == ":")
    );

    let in_terminal = in_force(cx, &TERMINAL);
    assert!(in_terminal.contains(&("terminal::Copy", format!("{CMD}-c"))));
    assert!(
        !in_terminal
            .iter()
            .any(|(a, _)| a.starts_with("resource_table::"))
    );
    assert!(
        !in_terminal
            .iter()
            .any(|(_, keys)| keys == ":" || keys == "ctrl-d")
    );
    // Listed once each, in a stable order.
    let mut sorted = in_terminal.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), in_terminal.len());
}
