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

use std::collections::{HashMap, HashSet};

use gpui::{App, KeyBinding, KeyContext, Keystroke, is_no_action, is_unbind};

use crate::layer::KeymapLayer;
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
    // The bindings that apply here, by keystrokes. A keystroke bound once and never nulled is won
    // by that binding with no further question; only the contested ones go to GPUI's own
    // resolution (`bindings_for_input` scans the whole keymap, so asking it for every binding is
    // quadratic in the bindings in force).
    let mut by_keys: HashMap<String, Contest<'_>> = HashMap::new();
    for binding in keymap.bindings() {
        if !applies_in(binding, stack) {
            continue;
        }
        let contest = by_keys.entry(keys_text(binding)).or_default();
        if is_null(binding) {
            contest.nulled = true;
        } else {
            contest.bindings.push(binding);
        }
    }
    let mut out: Vec<ActiveBinding> = Vec::new();
    let mut seen: HashSet<(&'static str, String, Option<String>)> = HashSet::new();
    for contest in by_keys.values() {
        for binding in &contest.bindings {
            let wins = if contest.bindings.len() == 1 && !contest.nulled {
                true
            } else {
                let (winners, _) = keymap.bindings_for_input(binding.keystrokes(), stack);
                winners.first().is_some_and(|winner| {
                    winner.action().partial_eq(binding.action())
                        && winner.keystrokes() == binding.keystrokes()
                })
            };
            if !wins {
                continue;
            }
            let entry = active_entry(binding);
            let key = (
                entry.action,
                entry.binding.keystrokes_text(),
                entry.binding.context.clone(),
            );
            if seen.insert(key) {
                out.push(entry);
            }
        }
    }
    sort(&mut out);
    out
}

/// The bindings of one keystroke sequence that apply in a context stack.
#[derive(Default)]
struct Contest<'a> {
    bindings: Vec<&'a KeyBinding>,
    /// Whether a `null` for the same keystrokes applies too.
    nulled: bool,
}

/// A binding's keystrokes as one string (`ctrl-k ctrl-s`), the key of its group.
fn keys_text(binding: &KeyBinding) -> String {
    binding
        .keystrokes()
        .iter()
        .map(|keystroke| keystroke.unparse())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Whether `binding` is a `null` (`NoAction`, or an `Unbind`): it hides bindings, it runs nothing.
fn is_null(binding: &KeyBinding) -> bool {
    is_no_action(binding.action()) || is_unbind(binding.action())
}

/// Whether `binding`'s context predicate holds for `stack` (no predicate: everywhere).
fn applies_in(binding: &KeyBinding, stack: &[KeyContext]) -> bool {
    binding
        .predicate()
        .is_none_or(|predicate| predicate.depth_of(stack).is_some())
}

/// Every binding of the installed keymap that runs an action, whatever context it is limited to:
/// what the help overlay lists when nothing is focused (no context stack to filter by). A binding
/// a `null` of the same keystrokes and context (from a layer at least as strong) hides is left
/// out. Sorted like [`active_bindings`]. Walks the whole keymap, so call it when an overlay
/// opens, not per frame.
pub fn all_bindings(cx: &App) -> Vec<ActiveBinding> {
    let keymap = cx.key_bindings();
    let keymap = keymap.borrow();
    let nulls: Vec<&KeyBinding> = keymap.bindings().filter(|b| is_null(b)).collect();
    let mut out: Vec<ActiveBinding> = Vec::new();
    for binding in keymap.bindings() {
        if is_null(binding) {
            continue;
        }
        let hidden = nulls.iter().any(|null| {
            null.keystrokes() == binding.keystrokes()
                && null.predicate().map(|p| p.to_string())
                    == binding.predicate().map(|p| p.to_string())
                && rank(null) <= rank(binding)
        });
        if hidden {
            continue;
        }
        let entry = active_entry(binding);
        if !out.contains(&entry) {
            out.push(entry);
        }
    }
    sort(&mut out);
    out
}

/// A binding a stronger layer's `null` hides in a context stack: what the help overlay shows as
/// "unbound by you".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SuppressedBinding {
    /// The hidden binding's action name.
    pub action: &'static str,
    /// The hidden binding (its own layer, keystrokes and context).
    pub binding: BindingInfo,
    /// The layer whose `null` hides it: the user's file, or the vim layer.
    pub by: KeymapLayer,
}

/// The bindings that apply in `stack` and that a `null` of the user's file or of the vim layer
/// hides: no binding runs for their keystrokes there. A binding another one replaced is not
/// listed (that one is in [`active_bindings`]); nor is one hidden by a default `null` (the
/// shipped keymap keeps text keys out of text fields). Sorted by action name, then keystrokes.
pub fn suppressed_bindings(cx: &App, stack: &[KeyContext]) -> Vec<SuppressedBinding> {
    let keymap = cx.key_bindings();
    let keymap = keymap.borrow();
    let nulls: Vec<&KeyBinding> = keymap
        .bindings()
        .filter(|b| is_null(b) && applies_in(b, stack))
        .collect();
    if nulls.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<SuppressedBinding> = Vec::new();
    for binding in keymap.bindings() {
        if is_null(binding) || !applies_in(binding, stack) {
            continue;
        }
        let by = nulls
            .iter()
            .filter(|null| null.keystrokes() == binding.keystrokes() && rank(null) < rank(binding))
            .filter_map(|null| KeymapLayer::from_meta(null.meta()))
            .find(|layer| *layer != KeymapLayer::Default);
        let Some(by) = by else { continue };
        let (winners, _) = keymap.bindings_for_input(binding.keystrokes(), stack);
        if !winners.is_empty() {
            continue;
        }
        let entry = SuppressedBinding {
            action: binding.action().name(),
            binding: BindingInfo::of(binding),
            by,
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

/// A binding's strength: its layer's metadata index, smaller is stronger; a binding another crate
/// added with `cx.bind_keys` ranks below every layer.
fn rank(binding: &KeyBinding) -> usize {
    binding.meta().map_or(usize::MAX, |meta| meta.0 as usize)
}

fn active_entry(binding: &KeyBinding) -> ActiveBinding {
    ActiveBinding {
        action: binding.action().name(),
        binding: BindingInfo::of(binding),
    }
}

fn sort(out: &mut [ActiveBinding]) {
    out.sort_by(|a, b| {
        a.action
            .cmp(b.action)
            .then_with(|| a.binding.keystrokes.cmp(&b.binding.keystrokes))
            .then_with(|| a.binding.context.cmp(&b.binding.context))
    });
}
