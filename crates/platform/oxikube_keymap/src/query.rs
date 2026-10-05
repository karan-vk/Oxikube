//! Look up the bindings of an action, for the command palette (E11) and menus to show next to
//! it.

use gpui::{Action, App};
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
        .map(|binding| BindingInfo {
            keystrokes: binding
                .keystrokes()
                .iter()
                .map(|keystroke| keystroke.unparse())
                .collect(),
            context: binding.predicate().map(|predicate| predicate.to_string()),
            layer: KeymapLayer::from_meta(binding.meta()),
        })
        .collect()
}

/// [`bindings_for_action`] for an action given by name and optional data, as the palette and
/// agent tools know it. Empty when the name is unknown or the data is rejected.
pub fn bindings_for_action_name(cx: &App, name: &str, data: Option<Value>) -> Vec<BindingInfo> {
    match ActionRegistry::build(cx, name, data) {
        Ok(action) => bindings_for_action(cx, action.as_ref()),
        Err(_) => Vec::new(),
    }
}
