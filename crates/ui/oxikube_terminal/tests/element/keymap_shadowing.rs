//! No workspace or global binding shadows a key a focused terminal sends to its process (E09-U560).
//!
//! GPUI matches key bindings before the focused element sees the key, so a `ctrl-w` bound to
//! "close tab" in the `Workspace` context (or with no context) never reaches a shell that wants
//! it for "delete a word". The terminal's contract (see `oxikube_terminal::input`) is that every
//! plain `ctrl-` chord, and every alt chord with `terminal.option_as_meta`, reaches the process;
//! `ctrl-shift-<key>` is the application namespace. This test builds the keymap an off-macOS
//! build installs (the embedded defaults of Linux and Windows, the workspace's interim bindings,
//! the menu bindings) and asks GPUI's own matcher, from inside the context stack of a terminal
//! in a pane and in the bottom dock, about every key `to_esc_str` encodes.

use gpui::{Action as _, KeyBinding, KeyContext, Keymap, Keystroke, actions};
use oxikube_assets::{KeymapPlatform, default_keymap};
use oxikube_keymap::file::{KeymapAction, parse_keymap};
use oxikube_keymap::layer::KeymapLayer;
use oxikube_keymap::{KeyContextBuilder, contexts};
use oxikube_terminal::mappings::{KeyMode, to_esc_str};
use oxikube_workspace::actions::default_bindings as workspace_bindings;
use oxikube_workspace::window::menus::default_bindings as menu_bindings;

actions!(
    keymap_shadowing,
    [
        /// Stands in for every action of the embedded keymap files outside the `Terminal` context.
        Probe,
        /// Stands in for the actions of the `Terminal` sections of the embedded keymap files.
        TerminalProbe,
        /// Stands in for a binding of the old, conflicting keymap.
        OldProbe
    ]
);

const OFF_MACOS: [(KeymapPlatform, &str); 2] = [
    (KeymapPlatform::Linux, "linux"),
    (KeymapPlatform::Windows, "windows"),
];

/// The bindings an off-macOS build installs, with `Probe` standing in for the file entries.
fn installed_bindings(platform: KeymapPlatform) -> Vec<KeyBinding> {
    let parsed = parse_keymap(default_keymap(platform), KeymapLayer::Default).expect("parses");
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let mut bindings = workspace_bindings(false);
    bindings.extend(menu_bindings(false));
    for (_, section) in &parsed.sections {
        for (keys, value) in &section.bindings {
            // `null` unbinds; nothing to shadow with.
            if let Ok(KeymapAction::Action { .. }) = KeymapAction::from_json(value) {
                let context = section.context_expr();
                bindings.push(if context.is_some_and(|c| c.contains("Terminal")) {
                    KeyBinding::new(keys, TerminalProbe, context)
                } else {
                    KeyBinding::new(keys, Probe, context)
                });
            }
        }
    }
    bindings
}

/// The key contexts from the workspace root to a focused terminal.
fn terminal_stacks(os: &str) -> Vec<(&'static str, Vec<KeyContext>)> {
    let build = |name: &str, extend: &dyn Fn(&mut KeyContextBuilder)| {
        let mut builder = KeyContextBuilder::new(name.to_owned());
        builder.value("os", os.to_owned());
        extend(&mut builder);
        builder.build()
    };
    let workspace = build(contexts::WORKSPACE, &|_| {});
    let terminal = build(contexts::TERMINAL, &|_| {});
    vec![
        (
            "a terminal in a pane",
            vec![
                workspace.clone(),
                build(contexts::PANE, &|_| {}),
                terminal.clone(),
            ],
        ),
        (
            "a terminal in the bottom dock",
            vec![
                workspace,
                build(contexts::DOCK, &|b| {
                    b.value("position", "bottom");
                }),
                terminal,
            ],
        ),
    ]
}

/// Every keystroke a terminal encodes for its process: plain ctrl chords, ctrl+alt chords and
/// (with `option_as_meta`) alt chords, over the printable keys and the named ones.
fn forwarded_keystrokes(mode: KeyMode) -> Vec<Keystroke> {
    let printable = (0x21u8..=0x7e).map(|b| (b as char).to_string());
    let named = [
        "space",
        "tab",
        "enter",
        "escape",
        "backspace",
        "delete",
        "insert",
        "home",
        "end",
        "pageup",
        "pagedown",
        "up",
        "down",
        "left",
        "right",
    ]
    .map(str::to_owned);
    let function = (1..=12).map(|n| format!("f{n}"));
    let mut out = Vec::new();
    for key in printable.chain(named).chain(function) {
        // `A` parses as shift-a: the shifted chords are the application namespace.
        if key.chars().any(|c| c.is_ascii_uppercase()) {
            continue;
        }
        for modifiers in ["ctrl", "ctrl-alt", "alt"] {
            // Written as GPUI parses a chord; `-` itself is the key of `ctrl--`.
            let Ok(keystroke) = Keystroke::parse(&format!("{modifiers}-{key}")) else {
                continue;
            };
            if to_esc_str(&keystroke, mode).is_some() {
                out.push(keystroke);
            }
        }
    }
    out
}

fn describe(binding: &KeyBinding) -> String {
    let keys: Vec<String> = binding
        .keystrokes()
        .iter()
        .map(|k| k.inner().unparse())
        .collect();
    format!(
        "`{}` ({}, context {:?})",
        keys.join(" "),
        binding.action().name(),
        binding.predicate().map(|_| "scoped")
    )
}

#[test]
fn no_binding_shadows_a_key_the_terminal_forwards() {
    let mode = KeyMode {
        app_cursor: false,
        option_as_meta: true,
    };
    let forwarded = forwarded_keystrokes(mode);
    assert!(
        forwarded.len() > 100,
        "the sweep covers the mapping table ({} keystrokes)",
        forwarded.len()
    );
    let mut shadowed = Vec::new();
    for (platform, os) in OFF_MACOS {
        let keymap = Keymap::new(installed_bindings(platform));
        for (place, stack) in terminal_stacks(os) {
            for keystroke in &forwarded {
                let (bound, pending) =
                    keymap.bindings_for_input(std::slice::from_ref(keystroke), &stack);
                for binding in &bound {
                    shadowed.push(format!(
                        "{os}, {place}: {} is bound to {}",
                        keystroke.unparse(),
                        describe(binding)
                    ));
                }
                if pending {
                    shadowed.push(format!(
                        "{os}, {place}: {} starts a chord",
                        keystroke.unparse()
                    ));
                }
            }
        }
    }
    shadowed.sort();
    shadowed.dedup();
    assert!(
        shadowed.is_empty(),
        "these bindings shadow keys the terminal sends to its process; move them to \
         ctrl-shift-<key> or scope them `!Terminal`:\n{}",
        shadowed.join("\n")
    );
}

/// The check can fail: bindings the way they were before U560 are caught.
#[test]
fn the_sweep_catches_the_old_bindings() {
    let old = [
        KeyBinding::new("ctrl-w", OldProbe, Some("Workspace")),
        KeyBinding::new("ctrl-k left", OldProbe, Some("Workspace")),
        KeyBinding::new("ctrl-q", OldProbe, None),
        KeyBinding::new("ctrl--", OldProbe, None),
    ];
    let keymap = Keymap::new(old.to_vec());
    let mode = KeyMode {
        app_cursor: false,
        option_as_meta: true,
    };
    let forwarded = forwarded_keystrokes(mode);
    let (_, stack) = terminal_stacks("linux").remove(0);
    let caught = |keys: &str| {
        let keystroke = Keystroke::parse(keys).unwrap();
        assert!(forwarded.contains(&keystroke), "{keys} is forwarded");
        let (bound, pending) = keymap.bindings_for_input(&[keystroke], &stack);
        !bound.is_empty() || pending
    };
    assert!(caught("ctrl-w"));
    assert!(caught("ctrl-k"), "the chord prefix");
    assert!(caught("ctrl-q"));
    assert!(caught("ctrl--"));
}

/// The terminal's own `ctrl-shift-` chords keep working inside a terminal, over the workspace's
/// chords on the same keys (`ctrl-shift-k` is also the workspace's split prefix).
#[test]
fn the_terminals_own_chords_win_inside_it() {
    for (platform, os) in OFF_MACOS {
        let keymap = Keymap::new(installed_bindings(platform));
        for (place, stack) in terminal_stacks(os) {
            for chord in ["ctrl-shift-k", "ctrl-shift-w", "ctrl-shift-t"] {
                let keystroke = Keystroke::parse(chord).unwrap();
                let (bound, pending) = keymap.bindings_for_input(&[keystroke], &stack);
                assert_eq!(
                    bound.first().map(|b| b.action().name()),
                    Some(TerminalProbe.name()),
                    "{os}, {place}: {chord} is the terminal's"
                );
                assert!(!pending, "{os}, {place}: {chord} is not a pending chord");
            }
        }
    }
}

/// GPUI lets a key through to the focused element when nothing handles the action its binding
/// names, so a bound key only shadows a terminal key when a handler exists. The app has them.
fn handle_the_application_actions(cx: &mut gpui::App) {
    use oxikube_workspace::actions::{
        CloseActiveItem, ReopenClosedItem, SplitDown, SplitLeft, SplitRight, SplitUp,
        ToggleBottomDock, ToggleLeftDock, ToggleRightDock, ToggleZoom,
    };
    use oxikube_workspace::session::{NewWindow, Quit, ZoomIn, ZoomOut, ZoomReset};
    macro_rules! swallow {
        ($($action:ty),* $(,)?) => {
            $(cx.on_action(|_: &$action, _| {});)*
        };
    }
    swallow!(
        CloseActiveItem,
        ReopenClosedItem,
        SplitDown,
        SplitLeft,
        SplitRight,
        SplitUp,
        ToggleBottomDock,
        ToggleLeftDock,
        ToggleRightDock,
        ToggleZoom,
        NewWindow,
        Quit,
        ZoomIn,
        ZoomOut,
        ZoomReset,
    );
}

/// The same, end to end: the off-macOS keymap installed in a window whose focused terminal sits
/// under a `Workspace` context. The old shadowed keys reach the process; the application chords
/// do not, and the terminal's own chord still wins over the workspace's on the same key.
#[gpui::test]
fn the_shell_keys_reach_the_process_under_the_off_macos_keymap(cx: &mut gpui::TestAppContext) {
    for (platform, os) in OFF_MACOS {
        cx.update(|cx| {
            handle_the_application_actions(cx);
            cx.bind_keys(workspace_bindings(false));
            cx.bind_keys(menu_bindings(false));
            oxikube_keymap::init_with_text(
                "[]",
                oxikube_keymap::KeymapOptions {
                    platform,
                    vim: false,
                },
                cx,
            );
        });
        let mut h = super::harness(cx, 480., 130., super::FONT_SIZE);
        h.window
            .simulate_keystrokes("ctrl-w ctrl-b ctrl-j ctrl-k ctrl-q ctrl-4 ctrl-alt-b ctrl--");
        assert_eq!(
            h.written(),
            b"\x17\x02\n\x0b\x11\x1c\x1b\x02\x1f",
            "{os}: the keys a shell uses reach it"
        );
        // ctrl-shift-k is the terminal's clear, not the workspace's split prefix: nothing is
        // pending afterwards, so the next key is an ordinary one.
        h.window.simulate_keystrokes("ctrl-shift-k ctrl-w");
        assert_eq!(
            h.written().last(),
            Some(&0x17),
            "{os}: no chord was left pending"
        );
        assert_eq!(h.commands().len(), 0, "{os}: no application command ran");
    }
}
