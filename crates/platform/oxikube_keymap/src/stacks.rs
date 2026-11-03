//! Where each view context sits: the focus stacks (outermost first) the app's views produce, the
//! context hierarchy the keymap editor's conflict check walks (E21-S08).
//!
//! GPUI matches a binding against the context stack of the focused element, so two bindings of
//! the same keys can only clash when one stack can satisfy both predicates: equal contexts, or
//! one an ancestor of the other (`ResourceTable` inside `ClusterTab` inside `Workspace`), never
//! two siblings (`ResourceTable` and `Terminal`). This module is that knowledge as data, beside
//! the context names of [`crate::known`] (the registry the validator uses): a process-wide list
//! seeded with the standard stacks ([`SEED_STACKS`]) that a view adds its own to with
//! [`register_stack`]. A context no stack holds (a view that registered only its name, a typo
//! in the user's file) is assumed to sit where most views do, in a pane of a cluster tab
//! ([`IN_CLUSTER_TAB`]).

use std::sync::{LazyLock, RwLock};

use crate::context::contexts;
use crate::known::KnownNames;

/// The stack above a view in a pane of a cluster tab: the window's workspace, the dock holding
/// the cluster tabs, the tab, the tab's own workspace and the pane.
pub const IN_CLUSTER_TAB: &[&str] = &[
    contexts::WORKSPACE,
    contexts::DOCK,
    contexts::CLUSTER_TAB,
    contexts::WORKSPACE,
    contexts::PANE,
];

/// The standard focus stacks, outermost first, past the generic [`IN_CLUSTER_TAB`] one each
/// known context gets.
pub const SEED_STACKS: &[&[&str]] = &[
    &[contexts::WORKSPACE],
    &[contexts::WORKSPACE, contexts::DOCK, contexts::CLUSTER_TAB],
    // The drawer of a cluster tab sits beside its workspace; a pinned detail is a pane item.
    &[
        contexts::WORKSPACE,
        contexts::DOCK,
        contexts::CLUSTER_TAB,
        contexts::DETAIL_DRAWER,
    ],
    // Text fields inside views.
    &[
        contexts::WORKSPACE,
        contexts::DOCK,
        contexts::CLUSTER_TAB,
        contexts::WORKSPACE,
        contexts::PANE,
        contexts::RESOURCE_TABLE,
        "Input",
    ],
    &[
        contexts::WORKSPACE,
        contexts::DOCK,
        contexts::CLUSTER_TAB,
        contexts::WORKSPACE,
        contexts::PANE,
        contexts::LOGS,
        "Input",
    ],
    &[
        contexts::WORKSPACE,
        contexts::DOCK,
        contexts::CLUSTER_TAB,
        contexts::WORKSPACE,
        contexts::PANE,
        contexts::TERMINAL,
        "TerminalSearch",
    ],
    // The catalog home is outside every cluster tab.
    &[contexts::WORKSPACE, contexts::CATALOG],
    &[contexts::WORKSPACE, contexts::CATALOG, "Input"],
    &[
        contexts::WORKSPACE,
        contexts::CATALOG,
        contexts::CATALOG_CLOUD,
    ],
    // Modals over the window, and over a cluster tab's own workspace.
    &[
        contexts::WORKSPACE,
        "ModalLayer",
        contexts::PALETTE,
        contexts::PICKER,
        "Input",
    ],
    &[
        contexts::WORKSPACE,
        "ModalLayer",
        contexts::JUMP_BAR,
        contexts::PICKER,
        "Input",
    ],
    &[
        contexts::WORKSPACE,
        "ModalLayer",
        contexts::HELP,
        contexts::PICKER,
        "Input",
    ],
    &[contexts::WORKSPACE, "ModalLayer", contexts::MODAL],
    &[
        contexts::WORKSPACE,
        "ModalLayer",
        "DialogModal",
        "DialogButton",
    ],
    &[
        contexts::WORKSPACE,
        contexts::DOCK,
        contexts::CLUSTER_TAB,
        contexts::WORKSPACE,
        "ModalLayer",
        contexts::MODAL,
    ],
    &[
        contexts::WORKSPACE,
        contexts::DOCK,
        contexts::CLUSTER_TAB,
        contexts::WORKSPACE,
        "ModalLayer",
        "DialogModal",
        "DialogButton",
    ],
    // The keymap editor's key recorder (E21-S08), over the page's cluster tab.
    &[
        contexts::WORKSPACE,
        contexts::DOCK,
        contexts::CLUSTER_TAB,
        contexts::WORKSPACE,
        "ModalLayer",
        "KeymapRecorder",
    ],
];

static REGISTRY: LazyLock<RwLock<Vec<Vec<String>>>> = LazyLock::new(|| {
    RwLock::new(
        SEED_STACKS
            .iter()
            .map(|stack| stack.iter().map(|name| (*name).to_owned()).collect())
            .collect(),
    )
});

/// Make `stack` (context names, outermost first) a known focus stack. Idempotent; a view whose
/// context does not sit in a pane of a cluster tab calls it when it is set up.
pub fn register_stack(stack: &[&str]) {
    let stack: Vec<String> = stack.iter().map(|name| (*name).to_owned()).collect();
    if let Ok(mut stacks) = REGISTRY.write()
        && !stacks.contains(&stack)
    {
        stacks.push(stack);
    }
}

/// A snapshot of the known focus stacks, `Send` and cheap to keep.
#[derive(Clone, Debug)]
pub struct ContextStacks {
    stacks: Vec<Vec<String>>,
}

impl ContextStacks {
    /// The registered stacks, plus the generic one ([`IN_CLUSTER_TAB`]) for every known context
    /// no registered stack holds.
    pub fn current() -> Self {
        let registered = REGISTRY
            .read()
            .map(|stacks| stacks.clone())
            .unwrap_or_else(|poisoned| poisoned.into_inner().clone());
        let mut stacks = Self { stacks: registered };
        for context in KnownNames::current().contexts() {
            stacks.ensure(context);
        }
        stacks
    }

    /// Exactly `stacks`, for tests.
    pub fn of(stacks: &[&[&str]]) -> Self {
        Self {
            stacks: stacks
                .iter()
                .map(|stack| stack.iter().map(|name| (*name).to_owned()).collect())
                .collect(),
        }
    }

    /// Add the generic stack for `context` unless a stack already holds it.
    pub fn ensure(&mut self, context: &str) {
        if self.has_context(context) {
            return;
        }
        let mut stack: Vec<String> = IN_CLUSTER_TAB
            .iter()
            .map(|name| (*name).to_owned())
            .collect();
        stack.push(context.to_owned());
        self.stacks.push(stack);
    }

    /// The stacks, outermost first within each.
    pub fn iter(&self) -> impl Iterator<Item = &[String]> {
        self.stacks.iter().map(Vec::as_slice)
    }

    /// Whether `name` is a view context of some stack (not a flag).
    pub fn has_context(&self, name: &str) -> bool {
        self.stacks
            .iter()
            .any(|stack| stack.iter().any(|n| n == name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_known_context_is_in_some_stack_and_views_can_add_theirs() {
        let stacks = ContextStacks::current();
        for context in KnownNames::current().contexts() {
            assert!(stacks.has_context(context), "{context}");
        }
        register_stack(&["Workspace", "StacksTestView"]);
        register_stack(&["Workspace", "StacksTestView"]);
        let again = ContextStacks::current();
        assert_eq!(
            again
                .iter()
                .filter(|s| s.last().is_some_and(|l| l == "StacksTestView"))
                .count(),
            1
        );
    }
}
