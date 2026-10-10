//! Look up the bindings of an action, for the command palette (E11) and menus to show next to
//! it.

use gpui::{Action, App, KeyBinding};
use oxikube_domain::command::CommandId;
use serde_json::Value;

use crate::layer::KeymapLayer;
use crate::registry::ActionRegistry;

/// One effective key binding of an action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BindingInfo {
    /// The keystrokes of the sequence, each in the keymap's own spelling (`cmd-k`).
    pub keystrokes: Vec<String>,
    /// The key-context expression the binding is limited to, `None` when it applies everywhere.
    pub context: Option<String>,
    /// The layer it came from; `None` for bindings other crates added with `cx.bind_keys`.
    pub layer: Option<KeymapLayer>,
}

impl BindingInfo {
    /// The description of one GPUI binding.
    pub fn of(binding: &KeyBinding) -> Self {
        Self {
            keystrokes: binding
                .keystrokes()
                .iter()
                .map(|keystroke| keystroke.unparse())
                .collect(),
            context: binding.predicate().map(|predicate| predicate.to_string()),
            layer: KeymapLayer::from_meta(binding.meta()),
        }
    }

    /// The keystrokes joined by spaces, as written in `keymap.json` (`ctrl-k ctrl-s`).
    pub fn keystrokes_text(&self) -> String {
        self.keystrokes.join(" ")
    }
}

/// The bindings that run `action`, strongest first (the first one is what a palette shows).
/// Bindings hidden by a `null` are not listed.
pub fn bindings_for_action(cx: &App, action: &dyn Action) -> Vec<BindingInfo> {
    let keymap = cx.key_bindings();
    let keymap = keymap.borrow();
    keymap
        .bindings_for_action(action)
        .rev()
        .map(BindingInfo::of)
        .collect()
}

/// The bindings that run `command`: those of the action named like it, then those of the view
/// actions that stand for it ([`stands_for`](crate::stands_for)), each list strongest first. What
/// the palette (E11-S03) shows next to a command; empty when nothing is bound.
pub fn bindings_for_command(cx: &App, command: CommandId) -> Vec<BindingInfo> {
    let mut out = bindings_for_action_name(cx, command.as_str(), None);
    for name in crate::stands_for::view_actions_of(command) {
        for info in bindings_for_action_name(cx, name, None) {
            if !out.contains(&info) {
                out.push(info);
            }
        }
    }
    out
}

/// [`bindings_for_action`] for an action given by name and optional data, as the palette and
/// agent tools know it. Empty when the name is unknown or the data is rejected.
pub fn bindings_for_action_name(cx: &App, name: &str, data: Option<Value>) -> Vec<BindingInfo> {
    match ActionRegistry::build(cx, name, data) {
        Ok(action) => bindings_for_action(cx, action.as_ref()),
        Err(_) => Vec::new(),
    }
}
