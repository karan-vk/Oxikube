//! Resolving a keystroke against a stack of key contexts without a window.
//!
//! GPUI dispatches a key to the focused element's ancestors, innermost first, and picks the
//! binding whose context predicate matches deepest ([`gpui::Keymap::bindings_for_input`]). The
//! functions here ask the same question of the installed keymap for a stack written down as data
//! (`["Workspace", "ClusterTab", "ResourceTable kind=Pod"]`), which is what the help overlay
//! (E11-S10) needs to list what a view's keys do, what the defaults tests need to check a key in
//! every Phase 1 context, and what a user keymap validator needs to report a conflict.
//!
//! Nothing here allocates per keystroke at dispatch time: this is for tools, tests and overlays,
//! run when asked for, never from the key path.

use gpui::{App, KeyContext, Keystroke, is_no_action, is_unbind};

use crate::query::BindingInfo;

/// What a key does in a context stack.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolution {
    /// The strongest matching binding runs `action`.
    Action {
        /// The action's registered name (`resource_table::ViewYaml`).
        name: &'static str,
        /// The binding that won.
        binding: BindingInfo,
    },
    /// The keystrokes are the start of a longer sequence bound here (`g` of `g g`).
    Pending,
    /// No binding applies, or a `null` hid the one that did: the focused view gets the key.
    Unbound,
}

impl Resolution {
    /// The action's name when the key runs one.
    pub fn action(&self) -> Option<&'static str> {
        match self {
            Self::Action { name, .. } => Some(name),
            Self::Pending | Self::Unbound => None,
        }
    }
}

/// A key context stack from its parts, outermost first. Each part is a `KeyContext` in GPUI's
/// source syntax: identifiers and `key=value` pairs (`"ResourceTable kind=Pod Editing"`).
///
/// # Errors
///
/// The first part that does not parse.
pub fn parse_stack(parts: &[&str]) -> Result<Vec<KeyContext>, String> {
    parts
        .iter()
        .map(|part| KeyContext::parse(part).map_err(|err| format!("context `{part}`: {err}")))
        .collect()
}

/// What `keystrokes` (one or more, space separated: `"g g"`) does with the installed keymap when
/// the focused element has the context stack `stack`, outermost first.
///
/// # Errors
///
/// A keystroke that does not parse.
pub fn resolve(cx: &App, keystrokes: &str, stack: &[KeyContext]) -> Result<Resolution, String> {
    let typed = keystrokes
        .split_whitespace()
        .map(|source| Keystroke::parse(source).map_err(|err| format!("`{source}`: {err}")))
        .collect::<Result<Vec<_>, _>>()?;
    let keymap = cx.key_bindings();
    let (bindings, pending) = keymap.borrow().bindings_for_input(&typed, stack);
    Ok(match bindings.first() {
        Some(binding) => Resolution::Action {
            name: binding.action().name(),
            binding: BindingInfo::of(binding),
        },
        None if pending => Resolution::Pending,
        None => Resolution::Unbound,
    })
}

/// A binding that is in force in a context stack.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveBinding {
    /// The action's registered name.
    pub action: &'static str,
    /// The keystrokes, the context expression and the layer.
    pub binding: BindingInfo,
}

/// The bindings in force when the focused element has the context stack `stack`: every binding
/// whose context matches and that no other binding or `null` outranks for its keystrokes. Sorted
/// by action name, then keystrokes, so the order is stable between calls.
///
/// This walks the whole keymap (a few hundred bindings), so call it when an overlay opens, not
/// per frame.
pub fn active_bindings(cx: &App, stack: &[KeyContext]) -> Vec<ActiveBinding> {
    let keymap = cx.key_bindings();
    let keymap = keymap.borrow();
    let mut out: Vec<ActiveBinding> = Vec::new();
    for binding in keymap.bindings() {
        let action = binding.action();
        if is_no_action(action) || is_unbind(action) {
            continue;
        }
        if binding
            .predicate()
            .is_some_and(|predicate| predicate.depth_of(stack).is_none())
        {
            continue;
        }
        let (winners, _) = keymap.bindings_for_input(binding.keystrokes(), stack);
        let wins = winners.first().is_some_and(|winner| {
            winner.action().partial_eq(action) && winner.keystrokes() == binding.keystrokes()
        });
        if !wins {
            continue;
        }
        let entry = ActiveBinding {
            action: action.name(),
            binding: BindingInfo::of(binding),
        };
        if !out.contains(&entry) {
            out.push(entry);
        }
    }
    out.sort_by(|a, b| {
        a.action
            .cmp(b.action)
            .then_with(|| a.binding.keystrokes.cmp(&b.binding.keystrokes))
    });
    out
}
