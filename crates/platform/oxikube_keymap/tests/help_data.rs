//! What the help overlay (E11-S10) reads from the installed keymap: every binding, and the ones a
//! user `null` hides.

use gpui::{TestAppContext, actions};
use oxikube_keymap::{
    KeymapLayer, KeymapOptions, KeymapPlatform, SuppressedBinding, active_bindings, all_bindings,
    init_with_text, parse_stack, suppressed_bindings,
};

actions!(
    resource_table,
    [
        ViewYaml,
        ViewDescribe,
        DeleteSelected,
        FocusFilter,
        ToggleWide
    ]
);
actions!(help, [Show]);
actions!(palette, [OpenJump]);

fn install(cx: &mut TestAppContext, user: &str, vim: bool) {
    cx.update(|cx| {
        init_with_text(
            user,
            KeymapOptions {
                platform: KeymapPlatform::MacOs,
                vim,
            },
            cx,
        )
    });
}

const TABLE: [&str; 3] = [
    "Workspace",
    "ClusterTab connected",
    "ResourceTable kind=Pod selection=one",
];

fn suppressed(cx: &mut TestAppContext, stack: &[&str]) -> Vec<SuppressedBinding> {
    cx.read(|cx| suppressed_bindings(cx, &parse_stack(stack).unwrap()))
}

#[gpui::test]
fn every_binding_is_listed_once_whatever_its_context(cx: &mut TestAppContext) {
    install(cx, "", false);
    let all = cx.read(all_bindings);
    let has = |action: &str, keys: &str| {
        all.iter()
            .any(|b| b.action == action && b.binding.keystrokes_text() == keys)
    };
    assert!(has("resource_table::ViewYaml", "y"), "a table key");
    assert!(has("help::Show", "?"), "a cluster tab key");
    assert!(has("palette::OpenJump", ":"));
    // Nulls are not bindings.
    assert!(all.iter().all(|b| !b.action.is_empty()));
    let mut keyed: Vec<_> = all
        .iter()
        .map(|b| {
            (
                b.action,
                b.binding.keystrokes_text(),
                b.binding.context.clone(),
            )
        })
        .collect();
    let before = keyed.len();
    keyed.sort();
    keyed.dedup();
    assert_eq!(keyed.len(), before, "listed once each");
}

#[gpui::test]
fn a_user_rebind_replaces_the_default_in_the_list(cx: &mut TestAppContext) {
    install(
        cx,
        r#"[{"context": "ResourceTable && !Editing", "bindings": {"y": null, "x": "resource_table::ViewYaml"}}]"#,
        false,
    );
    let active = cx.read(|cx| active_bindings(cx, &parse_stack(&TABLE).unwrap()));
    let yaml: Vec<_> = active
        .iter()
        .filter(|b| b.action == "resource_table::ViewYaml")
        .collect();
    assert_eq!(yaml.len(), 1, "{yaml:?}");
    assert_eq!(yaml[0].binding.keystrokes_text(), "x");
    assert_eq!(yaml[0].binding.layer, Some(KeymapLayer::User));

    let all = cx.read(all_bindings);
    assert!(
        !all.iter()
            .any(|b| b.action == "resource_table::ViewYaml" && b.binding.keystrokes_text() == "y"),
        "the user's null of the same context hides the default from the full list too"
    );
}

#[gpui::test]
fn a_user_null_is_reported_as_the_binding_it_hid(cx: &mut TestAppContext) {
    install(
        cx,
        r#"[{"context": "ResourceTable && !Editing", "bindings": {"d": null}}]"#,
        false,
    );
    let hidden = suppressed(cx, &TABLE);
    assert_eq!(hidden.len(), 1, "{hidden:?}");
    assert_eq!(hidden[0].action, "resource_table::ViewDescribe");
    assert_eq!(hidden[0].binding.keystrokes_text(), "d");
    assert_eq!(hidden[0].by, KeymapLayer::User);
    // Not in force there ...
    let active = cx.read(|cx| active_bindings(cx, &parse_stack(&TABLE).unwrap()));
    assert!(
        !active
            .iter()
            .any(|b| b.action == "resource_table::ViewDescribe")
    );
    // ... and nothing is reported in a stack the null does not reach.
    assert!(suppressed(cx, &["Workspace", "Terminal"]).is_empty());
}

#[gpui::test]
fn a_replaced_binding_and_the_shipped_text_nulls_are_not_reported(cx: &mut TestAppContext) {
    install(
        cx,
        r#"[{"context": "ResourceTable && !Editing", "bindings": {"y": "resource_table::ViewDescribe"}}]"#,
        false,
    );
    assert!(suppressed(cx, &TABLE).is_empty(), "y runs something else");
    install(cx, "", false);
    // The default keymap nulls `?` and `:` where text is typed; that is not "unbound by you".
    let typing = ["Workspace", "ClusterTab", "Input"];
    assert!(suppressed(cx, &typing).is_empty());
}

/// The definition `active_bindings` shortcuts: a binding is in force when GPUI's own resolution of
/// its keystrokes in the stack picks it.
fn reference(cx: &gpui::App, stack: &[gpui::KeyContext]) -> Vec<(String, String, Option<String>)> {
    let keymap = cx.key_bindings();
    let keymap = keymap.borrow();
    let mut out = Vec::new();
    for binding in keymap.bindings() {
        let action = binding.action();
        if gpui::is_no_action(action) || gpui::is_unbind(action) {
            continue;
        }
        if binding
            .predicate()
            .is_some_and(|predicate| predicate.depth_of(stack).is_none())
        {
            continue;
        }
        let (winners, _) = keymap.bindings_for_input(binding.keystrokes(), stack);
        if winners.first().is_some_and(|winner| {
            winner.action().partial_eq(action) && winner.keystrokes() == binding.keystrokes()
        }) {
            let info = oxikube_keymap::BindingInfo::of(binding);
            let row = (
                action.name().to_owned(),
                info.keystrokes_text(),
                info.context,
            );
            if !out.contains(&row) {
                out.push(row);
            }
        }
    }
    out.sort();
    out
}

#[gpui::test]
fn the_shortcut_agrees_with_gpuis_resolution_in_every_stack(cx: &mut TestAppContext) {
    let user = r#"[
        {"context": "ResourceTable && !Editing", "bindings": {"d": null, "x": "resource_table::ViewYaml", "ctrl-d": "resource_table::ViewDescribe"}},
        {"context": "Workspace", "bindings": {"ctrl-k ctrl-s": "help::Show"}}
    ]"#;
    for vim in [false, true] {
        install(cx, user, vim);
        for stack in [
            &["Workspace"][..],
            &[
                "Workspace",
                "ClusterTab connected",
                "Workspace",
                "Pane",
                "ResourceTable kind=Pod selection=one",
            ],
            &[
                "Workspace",
                "ClusterTab connected",
                "Workspace",
                "Pane",
                "ResourceTable kind=Pod Editing",
            ],
            &["Workspace", "ClusterTab connected", "LogView"],
            &["Workspace", "ClusterTab connected", "Terminal"],
            &["Workspace", "ClusterTab", "Input"],
            &[],
        ] {
            let stack = parse_stack(stack).unwrap();
            let mut fast: Vec<_> = cx.read(|cx| {
                active_bindings(cx, &stack)
                    .into_iter()
                    .map(|b| {
                        (
                            b.action.to_owned(),
                            b.binding.keystrokes_text(),
                            b.binding.context,
                        )
                    })
                    .collect()
            });
            fast.sort();
            let slow = cx.read(|cx| reference(cx, &stack));
            assert_eq!(fast, slow, "vim {vim}, stack {stack:?}");
        }
    }
}
