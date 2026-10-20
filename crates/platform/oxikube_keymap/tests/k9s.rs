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
        ShowServicePods,
        ShowUsedBy,
        OpenInBrowser,
        DeleteSelected,
        EditLabels,
        ShellSelected,
        ScaleSelected,
        RestartSelected,
        SetImageSelected,
        RollbackSelected,
        AttachSelected,
        CordonSelected,
        UncordonSelected,
        DrainSelected,
        FocusFilter,
        ClearFilter,
        ClearSelection,
        TriggerSelected,
        ToggleSuspendSelected,
        UseNamespace,
        ToggleFavouriteNamespace,
        KillSelected,
        SanitizeSelected,
        ShowNodeSelected,
        JumpToOwnerSelected,
        ExtendNext
    ]
);
actions!(resource_detail, [Close, Restart, SetImage]);
actions!(palette, [Toggle, OpenJump]);
actions!(help, [Show]);
actions!(terminal, [Copy, Split]);
actions!(log_view, [ToggleAutoscroll, ToggleWrap, Find]);

#[derive(Clone, PartialEq, Deserialize, JsonSchema, Action)]
#[action(namespace = resource_detail)]
struct ShowTab {
    index: u8,
}

#[derive(Clone, PartialEq, Deserialize, JsonSchema, Action)]
#[action(namespace = resource_table)]
struct SelectNamespaceSlot {
    slot: u8,
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

/// The node verbs (E12-S09): `c` cordons and `u` uncordons, on the Nodes table only, on every OS;
/// both are commands, neither is bound on another kind (`u` stays free for "used by"), and a
/// letter typed into the filter is text.
#[gpui::test]
fn c_and_u_cordon_and_uncordon_on_the_nodes_table_only(cx: &mut TestAppContext) {
    use oxikube_keymap::ActionRegistry;
    for platform in [MAC, LINUX] {
        install(cx, platform, "");
        let nodes = table("Node", "");
        assert_eq!(
            runs(cx, "c", &strs(&nodes)),
            Some("resource_table::CordonSelected"),
            "{platform:?}"
        );
        assert_eq!(
            runs(cx, "u", &strs(&nodes)),
            Some("resource_table::UncordonSelected"),
            "{platform:?}"
        );
        // The node shell keeps `s`.
        assert_eq!(
            runs(cx, "s", &strs(&nodes)),
            Some("resource_table::ShellSelected")
        );
        // `u` is used-by on five other kinds (`u_shows_what_uses_the_row_on_the_five_...`), free here.
        for kind in ["Pod", "Deployment", "Job"] {
            let other = table(kind, "");
            for keys in ["c", "u"] {
                assert_eq!(
                    press(cx, keys, &strs(&other)),
                    Resolution::Unbound,
                    "{keys} on {kind} ({platform:?})"
                );
            }
        }
        let typing = table("Node", "Editing");
        for keys in ["c", "u"] {
            assert_eq!(
                press(cx, keys, &strs(&typing)),
                Resolution::Unbound,
                "{keys}"
            );
        }
    }
    // Both keys stand for the commands of the same meaning (the palette shows them next to it).
    assert_eq!(
        ActionRegistry::commands_of("resource_table::CordonSelected"),
        [oxikube_domain::command::CommandId::NODE_CORDON]
    );
    assert_eq!(
        ActionRegistry::commands_of("resource_table::UncordonSelected"),
        [oxikube_domain::command::CommandId::NODE_UNCORDON]
    );
}

/// The drain verb (E12-S10): `r` on the Nodes table opens the drain dialog, on every OS; it stands
/// for `node::Drain`, is not bound on a Pods or Secrets table, and is text in the filter.
#[gpui::test]
fn r_drains_on_the_nodes_table_only(cx: &mut TestAppContext) {
    use oxikube_keymap::ActionRegistry;
    for platform in [MAC, LINUX] {
        install(cx, platform, "");
        let nodes = table("Node", "");
        assert_eq!(
            runs(cx, "r", &strs(&nodes)),
            Some("resource_table::DrainSelected"),
            "{platform:?}"
        );
        for kind in ["Pod", "Secret"] {
            let other = table(kind, "");
            assert_ne!(
                runs(cx, "r", &strs(&other)),
                Some("resource_table::DrainSelected"),
                "r on {kind} ({platform:?})"
            );
        }
        let typing = table("Node", "Editing");
        assert_eq!(press(cx, "r", &strs(&typing)), Resolution::Unbound);
    }
    assert_eq!(
        ActionRegistry::commands_of("resource_table::DrainSelected"),
        [oxikube_domain::command::CommandId::NODE_DRAIN]
    );
}

/// Used-by (E12-S18): `u` on the tables of a ServiceAccount, ConfigMap, Secret, claim and
/// PriorityClass shows what uses the row, on every OS; the other kinds keep `u` for their own verb
/// (Namespaces use it, Services show pods, Nodes uncordon) or leave it free, and a letter typed
/// into the filter is text.
#[gpui::test]
fn u_shows_what_uses_the_row_on_the_five_referenced_kinds_only(cx: &mut TestAppContext) {
    use oxikube_keymap::ActionRegistry;
    for platform in [MAC, LINUX] {
        install(cx, platform, "");
        for kind in [
            "ServiceAccount",
            "ConfigMap",
            "Secret",
            "PersistentVolumeClaim",
            "PriorityClass",
        ] {
            let stack = table(kind, "");
            assert_eq!(
                runs(cx, "u", &strs(&stack)),
                Some("resource_table::ShowUsedBy"),
                "{kind} ({platform:?})"
            );
            let typing = table(kind, "Editing");
            assert_eq!(
                press(cx, "u", &strs(&typing)),
                Resolution::Unbound,
                "{kind}"
            );
        }
        for (kind, own) in [
            ("Namespace", Some("resource_table::UseNamespace")),
            ("Service", Some("resource_table::ShowServicePods")),
            ("Node", Some("resource_table::UncordonSelected")),
            ("Pod", None),
            ("Deployment", None),
        ] {
            let stack = table(kind, "");
            assert_eq!(runs(cx, "u", &strs(&stack)), own, "{kind} ({platform:?})");
        }
    }
    assert_eq!(
        ActionRegistry::commands_of("resource_table::ShowUsedBy"),
        [CommandId::RESOURCE_SHOW_USED_BY]
    );
}

/// Verbs whose handler only toasts "not available yet", so they stand for no command. None at
/// present.
const PLACEHOLDER_VERBS: &[&str] = &[];

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
        // `?` is always useful: the catalog home opens the help overlay too, but its search field
        // keeps `?` as a character.
        assert_eq!(
            runs(cx, "shift-/->?", &["Workspace", "Catalog"]),
            Some("help::Show"),
            "{platform:?}"
        );
        assert_eq!(runs(cx, "?", &["Workspace", "Catalog"]), Some("help::Show"));
        assert_eq!(
            press(cx, "?", &["Workspace", "Catalog", "Input"]),
            Resolution::Unbound,
            "a character in the catalog search field"
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
    // E12 binds `s` to scale on workloads this way; the table's context carries `kind`.
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
fn cronjobs_trigger_on_t_and_cronjobs_and_jobs_suspend_on_s(cx: &mut TestAppContext) {
    // E12-S08: the k9s verbs `t` (trigger) and `s` (suspend) are bound for the kinds that have
    // them, so `s` stays a shell on pods and nodes.
    for platform in [MAC, LINUX] {
        install(cx, platform, "");
        let cronjobs = table("CronJob", "");
        let jobs = table("Job", "");
        let pods = table("Pod", "");
        assert_eq!(
            runs(cx, "t", &strs(&cronjobs)),
            Some("resource_table::TriggerSelected"),
            "{platform:?}"
        );
        assert_eq!(
            runs(cx, "s", &strs(&cronjobs)),
            Some("resource_table::ToggleSuspendSelected")
        );
        assert_eq!(
            runs(cx, "s", &strs(&jobs)),
            Some("resource_table::ToggleSuspendSelected")
        );
        // `t` triggers CronJobs only; a Job has nothing to trigger.
        assert_eq!(press(cx, "t", &strs(&jobs)), Resolution::Unbound);
        assert_eq!(press(cx, "t", &strs(&pods)), Resolution::Unbound);
        assert_eq!(
            runs(cx, "s", &strs(&pods)),
            Some("resource_table::ShellSelected")
        );
        // The other verbs of a CronJob table keep their meaning.
        assert_eq!(
            runs(cx, "y", &strs(&cronjobs)),
            Some("resource_table::ViewYaml")
        );
        // While the filter field has the focus, the keys are text.
        let typing = table("CronJob", "Editing");
        assert_eq!(press(cx, "t", &strs(&typing)), Resolution::Unbound);
        assert_eq!(press(cx, "s", &strs(&typing)), Resolution::Unbound);
    }
}

#[gpui::test]
fn a_services_verbs_are_bound_on_service_tables_only(cx: &mut TestAppContext) {
    // E12-S13: `u` shows the pods a Service selects, `o` opens a port in the browser.
    for platform in [MAC, LINUX] {
        install(cx, platform, "");
        let services = table("Service", "");
        assert_eq!(
            runs(cx, "u", &strs(&services)),
            Some("resource_table::ShowServicePods")
        );
        assert_eq!(
            runs(cx, "o", &strs(&services)),
            Some("resource_table::OpenInBrowser")
        );
        for kind in ["Pod", "Deployment", "Endpoints"] {
            let stack = table(kind, "");
            assert_eq!(runs(cx, "u", &strs(&stack)), None, "u on {kind}");
        }
        for kind in ["Deployment", "Endpoints"] {
            let stack = table(kind, "");
            assert_eq!(runs(cx, "o", &strs(&stack)), None, "o on {kind}");
        }
        // `o` is a Pod's own verb (E12-S03: show the node), never the Service's browser.
        assert_eq!(
            runs(cx, "o", &strs(&table("Pod", ""))),
            Some("resource_table::ShowNodeSelected")
        );
        // A Service table keeps the generic verbs.
        assert_eq!(
            runs(cx, "y", &strs(&services)),
            Some("resource_table::ViewYaml")
        );
        // Typing into the filter field stays text.
        let editing = table("Service", "Editing");
        assert_eq!(press(cx, "u", &strs(&editing)), Resolution::Unbound);
    }
}

#[gpui::test]
fn pods_kill_sanitize_show_their_node_and_jump_to_their_owner(cx: &mut TestAppContext) {
    // E12-S03: ctrl-k (force delete), z (sanitize), o (show node) and shift-j (jump to owner) are
    // Pod verbs; on other tables the keys stay unbound or keep their meaning.
    for platform in [MAC, LINUX] {
        install(cx, platform, "");
        let pods = table("Pod", "");
        let nodes = table("Node", "");
        let jobs = table("Job", "");
        assert_eq!(
            runs(cx, "ctrl-k", &strs(&pods)),
            Some("resource_table::KillSelected"),
            "{platform:?}"
        );
        assert_eq!(
            runs(cx, "z", &strs(&pods)),
            Some("resource_table::SanitizeSelected")
        );
        assert_eq!(
            runs(cx, "o", &strs(&pods)),
            Some("resource_table::ShowNodeSelected")
        );
        assert_eq!(
            runs(cx, "shift-j", &strs(&pods)),
            Some("resource_table::JumpToOwnerSelected")
        );
        // The other Pod verbs keep their meaning: s is a shell, a an attach, l the logs.
        assert_eq!(
            runs(cx, "s", &strs(&pods)),
            Some("resource_table::ShellSelected")
        );
        assert_eq!(
            runs(cx, "a", &strs(&pods)),
            Some("resource_table::AttachSelected")
        );
        assert_eq!(
            runs(cx, "l", &strs(&pods)),
            Some("resource_table::ViewLogs")
        );
        // Other kinds: no kill, sanitize or show node, and shift-j still extends the selection.
        for other in [&nodes, &jobs] {
            assert_eq!(press(cx, "ctrl-k", &strs(other)), Resolution::Unbound);
            assert_eq!(press(cx, "z", &strs(other)), Resolution::Unbound);
            assert_eq!(press(cx, "o", &strs(other)), Resolution::Unbound);
            assert_eq!(
                runs(cx, "shift-j", &strs(other)),
                Some("resource_table::ExtendNext")
            );
        }
        // While the filter field has the focus, the keys are text.
        let typing = table("Pod", "Editing");
        for key in ["ctrl-k", "z", "o"] {
            assert_eq!(press(cx, key, &strs(&typing)), Resolution::Unbound, "{key}");
        }
    }
}

#[gpui::test]
fn the_palette_finds_the_keys_of_the_pod_commands(cx: &mut TestAppContext) {
    install(cx, MAC, "");
    let keys = |cx: &mut TestAppContext, command| {
        cx.read(|cx| {
            bindings_for_command(cx, command)
                .into_iter()
                .map(|info| info.keystrokes_text())
                .collect::<Vec<_>>()
        })
    };
    assert_eq!(keys(cx, CommandId::POD_FORCE_DELETE), ["ctrl-k"]);
    assert_eq!(keys(cx, CommandId::POD_SANITIZE), ["z"]);
    assert_eq!(keys(cx, CommandId::POD_SHOW_NODE), ["o"]);
    assert_eq!(keys(cx, CommandId::POD_JUMP_TO_OWNER), ["shift-j"]);
    // Delete is bound once for every kind (`pod::Delete` and `resource::Delete` share the keys).
    let delete = keys(cx, CommandId::POD_DELETE);
    assert!(delete.iter().any(|k| k == "ctrl-d"), "{delete:?}");
}

#[gpui::test]
fn the_palette_finds_the_keys_of_the_cronjob_commands(cx: &mut TestAppContext) {
    install(cx, MAC, "");
    let keys = |cx: &mut TestAppContext, command| {
        cx.read(|cx| {
            bindings_for_command(cx, command)
                .into_iter()
                .map(|info| info.keystrokes_text())
                .collect::<Vec<_>>()
        })
    };
    assert_eq!(keys(cx, CommandId::CRONJOB_TRIGGER), ["t"]);
    // `s` is bound once for CronJob tables and once for Job tables: one key, two sections.
    for command in [
        CommandId::CRONJOB_SUSPEND,
        CommandId::CRONJOB_RESUME,
        CommandId::JOB_SUSPEND,
        CommandId::JOB_RESUME,
    ] {
        let found = keys(cx, command);
        assert!(
            !found.is_empty() && found.iter().all(|k| k == "s"),
            "{command}: {found:?}"
        );
    }
}

#[gpui::test]
fn the_service_verbs_stand_for_their_commands(cx: &mut TestAppContext) {
    use oxikube_keymap::ActionRegistry;
    install(cx, MAC, "");
    assert_eq!(
        ActionRegistry::commands_of("resource_table::ShowServicePods"),
        [CommandId::SERVICE_SHOW_PODS]
    );
    assert_eq!(
        ActionRegistry::commands_of("resource_table::OpenInBrowser"),
        [CommandId::SERVICE_OPEN_IN_BROWSER]
    );
}

#[gpui::test]
fn s_scales_the_workloads_that_can_scale_and_stays_a_shell_elsewhere(cx: &mut TestAppContext) {
    for platform in [MAC, LINUX] {
        install(cx, platform, "");
        for kind in [
            "Deployment",
            "StatefulSet",
            "ReplicaSet",
            "ReplicationController",
        ] {
            assert_eq!(
                runs(cx, "s", &strs(&table(kind, ""))),
                Some("resource_table::ScaleSelected"),
                "{kind} on {platform:?}"
            );
        }
        // A DaemonSet has no scale; a Pod and a Node keep the shell (a Job's `s` suspends it).
        for kind in ["DaemonSet", "Pod", "Node"] {
            assert_eq!(
                runs(cx, "s", &strs(&table(kind, ""))),
                Some("resource_table::ShellSelected"),
                "{kind} on {platform:?}"
            );
        }
        // Typing `s` into the table's filter field is text, not a scale.
        let editing = table("Deployment", "Editing");
        assert_eq!(press(cx, "s", &strs(&editing)), Resolution::Unbound);
    }
    // The palette shows the key next to the command.
    install(cx, MAC, "");
    let keys = cx.read(|cx| {
        bindings_for_command(cx, CommandId::WORKLOAD_SCALE)
            .into_iter()
            .map(|info| info.keystrokes_text())
            .collect::<Vec<_>>()
    });
    assert_eq!(keys, ["s"]);
}

#[gpui::test]
fn r_and_i_in_the_detail_drawer_act_on_the_workloads_a_template_change_rolls(
    cx: &mut TestAppContext,
) {
    let drawer = |kind: &str, extra: &str| {
        strs(&[
            "Workspace".to_owned(),
            "ClusterTab connected".to_owned(),
            "Workspace".to_owned(),
            "Dock position=right".to_owned(),
            format!("DetailDrawer mount=drawer kind={kind} {extra}"),
        ])
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>()
    };
    for platform in [MAC, LINUX] {
        install(cx, platform, "");
        for kind in ["Deployment", "StatefulSet", "DaemonSet"] {
            let stack = drawer(kind, "");
            assert_eq!(
                runs(cx, "r", &strs(&stack)),
                Some("resource_detail::Restart"),
                "{kind} on {platform:?}"
            );
            assert_eq!(
                runs(cx, "i", &strs(&stack)),
                Some("resource_detail::SetImage"),
                "{kind} on {platform:?}"
            );
        }
        for kind in ["ReplicaSet", "ReplicationController", "Pod", "Node"] {
            let stack = drawer(kind, "");
            assert_eq!(runs(cx, "r", &strs(&stack)), None, "{kind}");
            assert_eq!(runs(cx, "i", &strs(&stack)), None, "{kind}");
        }
        // The find field of the drawer has the focus: its keys are text.
        let editing = drawer("Deployment", "Editing");
        assert_eq!(press(cx, "r", &strs(&editing)), Resolution::Unbound);
        assert_eq!(press(cx, "i", &strs(&editing)), Resolution::Unbound);
    }
}

#[gpui::test]
fn r_restarts_and_i_sets_the_image_on_the_workloads_a_template_change_rolls(
    cx: &mut TestAppContext,
) {
    for platform in [MAC, LINUX] {
        install(cx, platform, "");
        for kind in ["Deployment", "StatefulSet", "DaemonSet"] {
            assert_eq!(
                runs(cx, "r", &strs(&table(kind, ""))),
                Some("resource_table::RestartSelected"),
                "{kind} on {platform:?}"
            );
            assert_eq!(
                runs(cx, "i", &strs(&table(kind, ""))),
                Some("resource_table::SetImageSelected"),
                "{kind} on {platform:?}"
            );
        }
        // A ReplicaSet's template change rolls nothing and a Pod has no template: neither restarts.
        for kind in ["ReplicaSet", "ReplicationController", "Pod", "Service"] {
            assert_eq!(runs(cx, "r", &strs(&table(kind, ""))), None, "{kind}");
            assert_eq!(runs(cx, "i", &strs(&table(kind, ""))), None, "{kind}");
        }
        // A Node's `r` is its drain (E12-S10), not a restart.
        let node = table("Node", "");
        assert_eq!(
            runs(cx, "r", &strs(&node)),
            Some("resource_table::DrainSelected")
        );
        assert_eq!(runs(cx, "i", &strs(&node)), None);
        // Typing into the table's filter field is text.
        let editing = table("Deployment", "Editing");
        assert_eq!(press(cx, "r", &strs(&editing)), Resolution::Unbound);
        assert_eq!(press(cx, "i", &strs(&editing)), Resolution::Unbound);
    }
    // The palette shows the keys next to the commands.
    install(cx, MAC, "");
    let keys = |cx: &mut TestAppContext, command| {
        cx.read(|cx| {
            bindings_for_command(cx, command)
                .into_iter()
                .map(|info| info.keystrokes_text())
                .collect::<Vec<_>>()
        })
    };
    assert_eq!(keys(cx, CommandId::WORKLOAD_RESTART), ["r"]);
    assert_eq!(keys(cx, CommandId::WORKLOAD_SET_IMAGE), ["i"]);
}

#[gpui::test]
fn ctrl_l_rolls_back_the_workloads_that_keep_a_history(cx: &mut TestAppContext) {
    for platform in [MAC, LINUX] {
        install(cx, platform, "");
        for kind in ["Deployment", "StatefulSet", "DaemonSet", "ReplicaSet"] {
            assert_eq!(
                runs(cx, "ctrl-l", &strs(&table(kind, ""))),
                Some("resource_table::RollbackSelected"),
                "{kind} on {platform:?}"
            );
        }
        // A ReplicationController has no history; a Pod's ctrl-l is not a rollback.
        for kind in ["ReplicationController", "Pod", "Node"] {
            assert_ne!(
                runs(cx, "ctrl-l", &strs(&table(kind, ""))),
                Some("resource_table::RollbackSelected"),
                "{kind} on {platform:?}"
            );
        }
        // Typing into the table's filter field is text, not a rollback.
        let editing = table("Deployment", "Editing");
        assert_eq!(press(cx, "ctrl-l", &strs(&editing)), Resolution::Unbound);
    }
    // The palette shows the key next to the command.
    install(cx, MAC, "");
    let keys = cx.read(|cx| {
        bindings_for_command(cx, CommandId::WORKLOAD_ROLLBACK)
            .into_iter()
            .map(|info| info.keystrokes_text())
            .collect::<Vec<_>>()
    });
    assert_eq!(keys, ["ctrl-l"]);
}

#[gpui::test]
fn shift_l_edits_the_labels_of_the_selection_on_every_kind_and_is_text_in_the_filter(
    cx: &mut TestAppContext,
) {
    for platform in [MAC, LINUX] {
        install(cx, platform, "");
        // Any kind: the bulk label editor works on a Pod, a Node and a custom resource alike.
        for kind in ["Pod", "Deployment", "Node", "ConfigMap", "Widget"] {
            assert_eq!(
                runs(cx, "shift-l", &strs(&table(kind, ""))),
                Some("resource_table::EditLabels"),
                "{kind} on {platform:?}"
            );
        }
        // The Delete key and `r` are the same keys with one row or several (the table turns them
        // into the bulk dialog): the bindings do not change with the selection.
        for selection in ["one", "many"] {
            let stack = vec![
                "Workspace".to_owned(),
                "ClusterTab connected".to_owned(),
                "Workspace".to_owned(),
                "Pane".to_owned(),
                format!("ResourceTable kind=Deployment selection={selection} scope=namespaced"),
            ];
            assert_eq!(
                runs(cx, "ctrl-d", &strs(&stack)),
                Some("resource_table::DeleteSelected")
            );
            assert_eq!(
                runs(cx, "r", &strs(&stack)),
                Some("resource_table::RestartSelected")
            );
        }
        // Typing into the table's filter field is text.
        let editing = table("Pod", "Editing");
        assert_eq!(press(cx, "shift-l", &strs(&editing)), Resolution::Unbound);
    }
    // The palette shows the key next to the command.
    install(cx, MAC, "");
    let keys = cx.read(|cx| {
        bindings_for_command(cx, CommandId::SELECTION_LABEL)
            .into_iter()
            .map(|info| info.keystrokes_text())
            .collect::<Vec<_>>()
    });
    assert_eq!(keys, ["shift-l"]);
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
    // `?` opens the help overlay from a cluster tab and from the catalog home and, in the
    // overlay's empty search field (E11-S10), closes it again: three bindings, one key.
    assert_eq!(keys(cx, CommandId::HELP_SHOW), ["?", "?", "?"]);
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

const SLOT: &str = "resource_table::SelectNamespaceSlot";

#[gpui::test]
fn the_namespace_keys_are_scoped_to_the_table_and_the_namespace_kind(cx: &mut TestAppContext) {
    for platform in [MAC, LINUX] {
        install(cx, platform, "");
        // On the Namespaces table: `u` uses the namespace under the cursor (k9s), `p` pins it.
        let namespaces = table("Namespace", "");
        assert_eq!(
            runs(cx, "u", &strs(&namespaces)),
            Some("resource_table::UseNamespace"),
            "{platform:?}"
        );
        assert_eq!(
            runs(cx, "p", &strs(&namespaces)),
            Some("resource_table::ToggleFavouriteNamespace")
        );
        // The digits select All (0) or the nth favourite, in any table.
        for kind in ["Namespace", "Pod", "Deployment"] {
            let stack = table(kind, "");
            for digit in 0..=9 {
                assert_eq!(
                    runs(cx, &digit.to_string(), &strs(&stack)),
                    Some(SLOT),
                    "{digit} on {kind} ({platform:?})"
                );
            }
        }
        // `u` and `p` are the Namespaces table's: other kinds keep them free (E12-S18 binds
        // used-by to `u` on its own kinds).
        for kind in ["Pod", "Deployment"] {
            let stack = table(kind, "");
            for keys in ["u", "p"] {
                assert_eq!(
                    press(cx, keys, &strs(&stack)),
                    Resolution::Unbound,
                    "{keys} on {kind}"
                );
            }
        }
    }
}

#[gpui::test]
fn the_namespace_keys_are_text_where_text_is_typed_and_keys_only_in_the_table(
    cx: &mut TestAppContext,
) {
    for platform in [MAC, LINUX] {
        install(cx, platform, "");
        let editing = table("Namespace", "Editing");
        let mut field = editing.clone();
        field.push("Input".into());
        for stack in [&editing, &field] {
            for keys in ["u", "p", "0", "1", "9"] {
                assert_eq!(
                    press(cx, keys, &strs(stack)),
                    Resolution::Unbound,
                    "{keys} typed into the filter ({platform:?})"
                );
            }
        }
        for keys in ["u", "p", "0", "1", "9"] {
            assert_eq!(
                press(cx, keys, &TERMINAL),
                Resolution::Unbound,
                "{keys} reaches the shell"
            );
        }
        // Outside a table the digit is that view's: the detail drawer's tabs, nothing at all in a
        // bare cluster tab.
        let drawer = [
            "Workspace",
            "ClusterTab connected",
            "Workspace",
            "Pane",
            "DetailDrawer kind=Namespace mount=drawer",
        ];
        assert_eq!(runs(cx, "1", &drawer), Some("resource_detail::ShowTab"));
        assert_eq!(
            press(
                cx,
                "1",
                &["Workspace", "ClusterTab connected", "Workspace", "Pane"]
            ),
            Resolution::Unbound
        );
        assert_eq!(
            press(cx, "1", &["Workspace", "Catalog"]),
            Resolution::Unbound
        );
    }
}

#[gpui::test]
fn the_namespace_actions_stand_for_commands(cx: &mut TestAppContext) {
    use oxikube_keymap::ActionRegistry;
    install(cx, MAC, "");
    for (action, command) in [
        ("resource_table::UseNamespace", CommandId::NAMESPACE_SELECT),
        (
            "resource_table::ToggleFavouriteNamespace",
            CommandId::NAMESPACE_TOGGLE_FAVOURITE,
        ),
        (SLOT, CommandId::NAMESPACE_USE_FAVOURITE),
        (
            "resource_table::DeleteSelected",
            CommandId::NAMESPACE_DELETE,
        ),
    ] {
        assert!(
            ActionRegistry::commands_of(action).contains(&command),
            "{action} stands for {command}"
        );
    }
}
