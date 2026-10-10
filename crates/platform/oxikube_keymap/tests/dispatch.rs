// Portions of this file are derived from Zed (https://github.com/zed-industries/zed),
// Copyright (c) Zed Industries, Inc. and contributors.
// Zed is licensed under the GNU General Public License v3.0 or later.
// Modifications Copyright (c) Oxikube contributors.
// SPDX-License-Identifier: GPL-3.0-or-later
// Source: crates/gpui/src/keymap.rs (the `tests` module) @ gpui-pre 0.3.7 (Zed's gpui snapshot)

//! Key dispatch precedence, adapted from Zed's keymap tests (E11-S07).
//!
//! The rules Oxikube's keymap layers lean on, asserted against GPUI's own `Keymap`: the deeper
//! context beats the shallower, a later binding beats an earlier one at equal depth, `null`
//! (`NoAction`) hides what it outranks and nothing from a stronger layer, multi-keystroke
//! prefixes stay pending exactly where a longer binding is live, and a source's rank is its
//! [`KeymapLayer::meta`] (user over vim over default).
//!
//! Zed's tests used editor/pane names and a four-layer source ranking (user, vim, base,
//! default); these use Oxikube's contexts (`Workspace`, `ClusterTab`, `ResourceTable`,
//! `Terminal`, ...) and its three layers. The assertions are Zed's, so a GPUI bump that changes
//! dispatch fails here before it changes what a key does in the app.

use gpui::{Action, KeyBinding, KeyContext, Keymap, Keystroke, NoAction, Unbind, actions};
use oxikube_keymap::KeymapLayer;

actions!(dispatch_test, [Alpha, Beta, Gamma]);

fn is<A: Action>(binding: &KeyBinding, action: &A) -> bool {
    binding.action().partial_eq(action)
}

fn key(source: &str) -> Keystroke {
    Keystroke::parse(source).unwrap()
}

fn stack(contexts: &[&str]) -> Vec<KeyContext> {
    contexts
        .iter()
        .map(|source| KeyContext::parse(source).unwrap())
        .collect()
}

fn build(bindings: impl IntoIterator<Item = KeyBinding>) -> Keymap {
    let mut keymap = Keymap::default();
    keymap.add_bindings(bindings);
    keymap
}

#[test]
fn a_binding_applies_only_where_its_context_matches() {
    let keymap = build([
        KeyBinding::new("ctrl-a", Alpha, None),
        KeyBinding::new("ctrl-a", Beta, Some("Pane")),
        KeyBinding::new("ctrl-a", Gamma, Some("ResourceTable && selection == one")),
    ]);
    let press = |contexts: &[&str]| {
        let (result, pending) = keymap.bindings_for_input(&[key("ctrl-a")], &stack(contexts));
        assert!(!pending);
        result
    };

    // A binding without a context applies everywhere, even with nothing focused.
    assert!(is(&press(&[])[0], &Alpha));
    assert!(is(&press(&["Terminal"])[0], &Alpha));
    // A contextual binding needs a matching context; `Pane` is not `Barf`.
    assert_eq!(press(&["Barf x=y"]).len(), 1);
    assert_eq!(press(&["Pane x=y"]).len(), 2);
    // Key/value pairs narrow a context: one selected row, not none.
    assert_eq!(press(&["ResourceTable"]).len(), 1);
    let one = press(&["ResourceTable selection=one"]);
    assert_eq!(one.len(), 2);
    assert!(is(&one[0], &Gamma));
}

#[test]
fn the_deeper_context_beats_the_shallower() {
    let keymap = build([
        KeyBinding::new("ctrl-a", Beta, Some("ClusterTab")),
        KeyBinding::new("ctrl-a", Gamma, Some("ResourceTable")),
    ]);
    let (result, pending) =
        keymap.bindings_for_input(&[key("ctrl-a")], &stack(&["ClusterTab", "ResourceTable"]));
    assert!(!pending);
    assert_eq!(result.len(), 2);
    assert!(is(&result[0], &Gamma), "the table's binding runs first");
    assert!(
        is(&result[1], &Beta),
        "the cluster tab's binding is the fallback"
    );
}

#[test]
fn at_equal_depth_the_later_binding_wins() {
    // A user section for the same context comes after the default one.
    let keymap = build([
        KeyBinding::new("cmd-r", Alpha, Some("Workspace")),
        KeyBinding::new("cmd-r", Beta, Some("Workspace")),
    ]);
    let (result, _) = keymap.bindings_for_input(&[key("cmd-r")], &stack(&["Workspace"]));
    assert!(is(&result[0], &Beta));
    assert!(is(&result[1], &Alpha));

    // Depth outranks position: an earlier binding in a deeper context still wins.
    let keymap = build([
        KeyBinding::new("cmd-r", Beta, Some("ResourceTable")),
        KeyBinding::new("cmd-r", Alpha, Some("Workspace")),
    ]);
    let (result, _) =
        keymap.bindings_for_input(&[key("cmd-r")], &stack(&["Workspace", "ResourceTable"]));
    assert!(
        is(&result[0], &Beta),
        "ResourceTable is deeper than Workspace"
    );
    assert!(is(&result[1], &Alpha));
}

#[test]
fn null_disables_a_binding_in_its_context_and_deeper_ones() {
    let keymap = build([
        KeyBinding::new("ctrl-a", Alpha, Some("ResourceTable")),
        KeyBinding::new("ctrl-b", Alpha, Some("ResourceTable")),
        KeyBinding::new(
            "ctrl-a",
            NoAction,
            Some("ResourceTable && selection == many"),
        ),
        KeyBinding::new("ctrl-b", NoAction, None),
    ]);
    let press = |source: &str, contexts: &[&str]| {
        keymap
            .bindings_for_input(&[key(source)], &stack(contexts))
            .0
            .len()
    };

    assert_eq!(press("ctrl-a", &["Barf"]), 0, "bound only in a table");
    assert_eq!(press("ctrl-a", &["ResourceTable"]), 1);
    assert_eq!(
        press("ctrl-a", &["ResourceTable selection=many"]),
        0,
        "hidden in the narrower context"
    );
    assert_eq!(
        press("ctrl-b", &["Barf"]),
        0,
        "a null without a context hides it everywhere"
    );
    assert_eq!(press("ctrl-b", &["ResourceTable"]), 0);
}

#[test]
fn a_null_in_a_deeper_context_hides_the_shallower_binding() {
    // The `:` jump bar is bound in `ClusterTab`; the shell, which types it, nulls it.
    let keymap = build([
        KeyBinding::new("shift-;", Alpha, Some("ClusterTab")),
        KeyBinding::new("shift-;", NoAction, Some("Terminal")),
    ]);
    let (result, pending) =
        keymap.bindings_for_input(&[key("shift-;")], &stack(&["ClusterTab", "Terminal"]));
    assert!(result.is_empty());
    assert!(!pending);
    let (result, _) = keymap.bindings_for_input(&[key("shift-;")], &stack(&["ClusterTab"]));
    assert_eq!(
        result.len(),
        1,
        "outside the terminal the key still opens the bar"
    );
}

#[test]
fn a_null_in_a_shallower_context_does_not_hide_a_deeper_binding() {
    let keymap = build([
        KeyBinding::new("ctrl-x", Alpha, Some("ResourceTable")),
        KeyBinding::new("ctrl-x", NoAction, Some("ClusterTab")),
    ]);
    let (result, pending) =
        keymap.bindings_for_input(&[key("ctrl-x")], &stack(&["ClusterTab", "ResourceTable"]));
    assert_eq!(result.len(), 1);
    assert!(!pending);
}

#[test]
fn a_null_for_a_sequence_hides_it_in_its_context_only() {
    // Zed issue 30259: `space w w` is bound in the workspace and nulled in the editor.
    let bindings = || {
        [
            KeyBinding::new("g g", Alpha, Some("ClusterTab")),
            KeyBinding::new("g g", NoAction, Some("Terminal")),
        ]
    };
    let keymap = build(bindings());
    let tab = || stack(&["ClusterTab"]);
    let terminal = || stack(&["ClusterTab", "Terminal"]);

    // `g` waits for a second key in the tab, not in the terminal.
    let (matched, pending) = keymap.bindings_for_input(&[key("g")], &tab());
    assert!(matched.is_empty());
    assert!(pending);
    let (matched, pending) = keymap.bindings_for_input(&[key("g")], &terminal());
    assert!(matched.is_empty());
    assert!(!pending);

    let g_g = [key("g"), key("g")];
    assert_eq!(keymap.bindings_for_input(&g_g, &tab()).0.len(), 1);
    assert!(keymap.bindings_for_input(&g_g, &terminal()).0.is_empty());

    // Another sequence with the same prefix, added before or after the null, or in the outer
    // context, keeps the prefix pending in the terminal.
    for bindings in [
        vec![
            KeyBinding::new("g g", Alpha, Some("ClusterTab")),
            KeyBinding::new("g g", NoAction, Some("Terminal")),
            KeyBinding::new("g t", Alpha, Some("Terminal")),
        ],
        vec![
            KeyBinding::new("g g", Alpha, Some("ClusterTab")),
            KeyBinding::new("g t", Alpha, Some("Terminal")),
            KeyBinding::new("g g", NoAction, Some("Terminal")),
        ],
        vec![
            KeyBinding::new("g g", Alpha, Some("ClusterTab")),
            KeyBinding::new("g t", Alpha, Some("ClusterTab")),
            KeyBinding::new("g g", NoAction, Some("Terminal")),
        ],
    ] {
        let keymap = build(bindings);
        let (matched, pending) = keymap.bindings_for_input(&[key("g")], &terminal());
        assert!(matched.is_empty());
        assert!(pending);
    }
}

#[test]
fn a_single_key_binding_and_a_longer_one_with_its_prefix() {
    // `ctrl-w left` is bound; nulling `ctrl-w` alone leaves the prefix pending, and a real
    // `ctrl-w` binding at the same depth takes the key at once.
    let nulled = build([
        KeyBinding::new("ctrl-w left", Alpha, Some("ResourceTable")),
        KeyBinding::new("ctrl-w", NoAction, Some("ResourceTable")),
    ]);
    let (result, pending) = nulled.bindings_for_input(&[key("ctrl-w")], &stack(&["ResourceTable"]));
    assert!(result.is_empty());
    assert!(pending);

    let bound = build([
        KeyBinding::new("ctrl-w left", Alpha, Some("ResourceTable")),
        KeyBinding::new("ctrl-w", Beta, Some("ResourceTable")),
    ]);
    let (result, pending) = bound.bindings_for_input(&[key("ctrl-w")], &stack(&["ResourceTable"]));
    assert_eq!(result.len(), 1);
    assert!(!pending);
}

#[test]
fn a_null_in_the_wrong_context_disables_nothing() {
    let keymap = build([
        KeyBinding::new("ctrl-x", Alpha, Some("ResourceTable")),
        KeyBinding::new("ctrl-x", NoAction, Some("Workspace")),
    ]);
    let (result, pending) =
        keymap.bindings_for_input(&[key("ctrl-x")], &stack(&["Workspace", "ResourceTable"]));
    assert_eq!(result.len(), 1, "the null is for the shallower context");
    assert!(!pending);
}

#[test]
fn pending_input_follows_the_contexts_the_longer_binding_is_live_in() {
    // `ctrl-x` is bound for filter-less tables; `ctrl-x 0` for the whole workspace.
    let keymap = build([
        KeyBinding::new("ctrl-x", Beta, Some("ResourceTable && !Editing")),
        KeyBinding::new("ctrl-x 0", Alpha, Some("Workspace")),
    ]);
    let focused = stack(&["Workspace", "Pane", "ResourceTable"]);
    let (matched, pending) = keymap.bindings_for_input(&[key("ctrl-x")], &focused);
    assert_eq!(matched.len(), 1);
    assert!(is(&matched[0], &Beta));
    assert!(pending, "the key could still be the start of ctrl-x 0");

    // A null on the longer sequence ends the wait.
    let keymap = build(vec![
        KeyBinding::new("ctrl-x", Beta, Some("ResourceTable && !Editing")),
        KeyBinding::new("ctrl-x 0", NoAction, Some("Workspace")),
    ]);
    let (matched, pending) = keymap.bindings_for_input(&[key("ctrl-x")], &focused);
    assert_eq!(matched.len(), 1);
    assert!(!pending);

    // A binding for the prefix that is deeper than the longer one overrides it.
    let keymap = build(vec![
        KeyBinding::new("ctrl-x 0", Alpha, Some("Workspace")),
        KeyBinding::new("ctrl-x", Beta, Some("ResourceTable && !Editing")),
    ]);
    let (matched, pending) = keymap.bindings_for_input(&[key("ctrl-x")], &focused);
    assert_eq!(matched.len(), 1);
    assert!(is(&matched[0], &Beta));
    assert!(!pending);
}

#[test]
fn a_layers_null_hides_weaker_layers_only() {
    let user = KeymapLayer::User.meta();
    let vim = KeymapLayer::Vim.meta();
    let default = KeymapLayer::Default.meta();
    let table = || stack(&["ResourceTable"]);
    let ctrl_x = || [key("ctrl-x")];

    // A vim null hides a default binding of the same context.
    let keymap = build([
        KeyBinding::new("ctrl-x", Alpha, Some("ResourceTable")).with_meta(default),
        KeyBinding::new("ctrl-x", NoAction, Some("ResourceTable")).with_meta(vim),
    ]);
    assert!(keymap.bindings_for_input(&ctrl_x(), &table()).0.is_empty());

    // A user binding is not hidden by a default's or the vim layer's null.
    let keymap = build(vec![
        KeyBinding::new("ctrl-x", NoAction, Some("ResourceTable")).with_meta(default),
        KeyBinding::new("ctrl-x", NoAction, Some("ResourceTable")).with_meta(vim),
        KeyBinding::new("ctrl-x", Beta, None).with_meta(user),
    ]);
    let (result, _) = keymap.bindings_for_input(&ctrl_x(), &table());
    assert_eq!(result.len(), 1);
    assert!(is(&result[0], &Beta));

    // A user binding in a shallower context survives a deeper default null.
    let keymap = build(vec![
        KeyBinding::new("ctrl-x", NoAction, Some("ResourceTable")).with_meta(default),
        KeyBinding::new("ctrl-x", Beta, Some("Workspace")).with_meta(user),
    ]);
    let (result, _) = keymap.bindings_for_input(&ctrl_x(), &stack(&["Workspace", "ResourceTable"]));
    assert_eq!(result.len(), 1);
    assert!(is(&result[0], &Beta));

    // A vim binding survives a default null, and a user null hides everything below it.
    let keymap = build(vec![
        KeyBinding::new("ctrl-x", Alpha, Some("ResourceTable")).with_meta(default),
        KeyBinding::new("ctrl-x", NoAction, Some("ResourceTable")).with_meta(default),
        KeyBinding::new("ctrl-x", Gamma, Some("ResourceTable")).with_meta(vim),
    ]);
    let (result, _) = keymap.bindings_for_input(&ctrl_x(), &table());
    assert_eq!(result.len(), 1);
    assert!(is(&result[0], &Gamma));

    let keymap = build(vec![
        KeyBinding::new("ctrl-x", Alpha, Some("ResourceTable")).with_meta(default),
        KeyBinding::new("ctrl-x", Gamma, Some("ResourceTable")).with_meta(vim),
        KeyBinding::new("ctrl-x", NoAction, Some("ResourceTable")).with_meta(user),
    ]);
    assert!(keymap.bindings_for_input(&ctrl_x(), &table()).0.is_empty());
}

#[test]
fn a_user_binding_outranks_a_default_one_in_the_same_context() {
    let mut default = KeyBinding::new("cmd-r", Alpha, Some("ResourceTable"));
    default.set_meta(KeymapLayer::Default.meta());
    let mut user = KeyBinding::new("cmd-r", Beta, Some("ResourceTable"));
    user.set_meta(KeymapLayer::User.meta());

    // Whichever order they are added in, the layers are merged default first; the user's
    // binding is the later one and comes first.
    let keymap = build([default, user]);
    let (result, _) = keymap.bindings_for_input(&[key("cmd-r")], &stack(&["ResourceTable"]));
    assert_eq!(result.len(), 2);
    assert!(is(&result[0], &Beta));
    assert!(is(&result[1], &Alpha));
}

#[test]
fn the_bindings_of_an_action_leave_out_the_ones_a_null_hides() {
    let keymap = build([
        KeyBinding::new("ctrl-a", Alpha, Some("ClusterTab")),
        KeyBinding::new("ctrl-b", Beta, Some("ResourceTable && selection == one")),
        KeyBinding::new("ctrl-c", Gamma, Some("Workspace")),
        KeyBinding::new("ctrl-a", NoAction, Some("ClusterTab && connected")),
        KeyBinding::new("ctrl-b", NoAction, Some("ResourceTable")),
    ]);
    let keystrokes = |action: &dyn Action| {
        keymap
            .bindings_for_action(action)
            .map(|binding| binding.keystrokes()[0].inner().unparse())
            .collect::<Vec<_>>()
    };
    // The null for `ClusterTab && connected` is narrower than the binding, so the binding stays.
    assert_eq!(keystrokes(&Alpha), ["ctrl-a"]);
    // The null for `ResourceTable` is broader than the binding's context.
    assert!(keystrokes(&Beta).is_empty());
    assert_eq!(keystrokes(&Gamma), ["ctrl-c"]);
}

#[test]
fn a_targeted_unbind_removes_one_action_and_leaves_the_others() {
    let keymap = build([
        KeyBinding::new("tab", Alpha, Some("ResourceTable")),
        KeyBinding::new("tab", Beta, Some("ResourceTable && selection == one")),
        KeyBinding::new(
            "tab",
            Unbind("dispatch_test::Alpha".into()),
            Some("ResourceTable && Editing"),
        ),
    ]);
    let (result, pending) = keymap.bindings_for_input(
        &[key("tab")],
        &stack(&["ResourceTable selection=one Editing"]),
    );
    assert!(!pending);
    assert_eq!(result.len(), 1);
    assert!(is(&result[0], &Beta));
}
