//! The names a `context` expression may use: view contexts (`ResourceTable`), flags (`Editing`)
//! and value keys (`kind`, `os`).
//!
//! The keymap validator warns about a name nobody knows (almost always a typo), so it needs the
//! list. The list is not hard-coded in the validator: it is a process-wide registry seeded with
//! the standard names ([`SEED_CONTEXTS`], [`SEED_FLAGS`]) that a view or an extension adds to
//! with [`register_context`] and [`register_flag`] when it is set up. [`KnownNames::current`]
//! takes a `Send` snapshot, so the editor can validate on a background executor.

use std::collections::BTreeSet;
use std::sync::{LazyLock, RwLock};

use crate::context::contexts;

/// The view contexts the app ships, including the ones only the default keymaps name.
pub const SEED_CONTEXTS: &[&str] = &[
    contexts::WORKSPACE,
    contexts::CLUSTER_TAB,
    contexts::DOCK,
    contexts::PANE,
    contexts::RESOURCE_TABLE,
    contexts::LIST,
    contexts::DETAIL_DRAWER,
    contexts::LOGS,
    contexts::TERMINAL,
    contexts::MANIFEST_EDITOR,
    contexts::PALETTE,
    contexts::PICKER,
    contexts::JUMP_BAR,
    contexts::HELP,
    contexts::MODAL,
    contexts::CATALOG,
    contexts::CATALOG_CLOUD,
    contexts::PULSES,
    contexts::EVENTS,
    contexts::RBAC_SUBJECTS,
    contexts::PORT_FORWARDS,
    contexts::HELM_RELEASES,
    contexts::APPLY_RESULTS,
    "Input",
    "NotificationPanel",
    "SettingsPage",
    "TerminalSearch",
    // Contexts views set with a literal or a view-local constant.
    "AuditView",
    "ClusterOverview",
    "ClusterSidebar",
    "ClusterSidebarSearch",
    "CodeView",
    "DialogButton",
    "DialogModal",
    "Hotbar",
    "KeymapRecorder",
    "ModalLayer",
    "NamespaceList",
    "NamespaceSearch",
    "NamespaceSelector",
    "Overview",
    "ToastLayer",
];

/// Flags and value keys the standard views set (see the table in [`crate::context`]).
pub const SEED_FLAGS: &[&str] = &[
    contexts::EDITING,
    "Loading",
    "autoscroll",
    "connected",
    "empty",
    "faults",
    "kind",
    "mode",
    "mount",
    "os",
    "paused",
    "position",
    "scope",
    "searching",
    "selection",
    "wrap",
];

#[derive(Clone, Debug)]
struct Names {
    contexts: BTreeSet<String>,
    flags: BTreeSet<String>,
}

static REGISTRY: LazyLock<RwLock<Names>> = LazyLock::new(|| {
    RwLock::new(Names {
        contexts: SEED_CONTEXTS
            .iter()
            .map(|name| (*name).to_owned())
            .collect(),
        flags: SEED_FLAGS.iter().map(|name| (*name).to_owned()).collect(),
    })
});

/// Make `name` a known view context. Idempotent; call it when the view is set up.
pub fn register_context(name: &str) {
    if let Ok(mut names) = REGISTRY.write() {
        names.contexts.insert(name.to_owned());
    }
}

/// Make `name` a known flag or value key. Idempotent.
pub fn register_flag(name: &str) {
    if let Ok(mut names) = REGISTRY.write() {
        names.flags.insert(name.to_owned());
    }
}

/// A snapshot of the known names, cheap to clone and `Send`.
#[derive(Clone, Debug)]
pub struct KnownNames {
    names: Names,
}

impl KnownNames {
    /// The names registered so far.
    pub fn current() -> Self {
        let names = REGISTRY
            .read()
            .map(|names| names.clone())
            .unwrap_or_else(|poisoned| poisoned.into_inner().clone());
        Self { names }
    }

    /// A snapshot of exactly `contexts` and `flags`, for tests.
    pub fn of<'a>(
        contexts: impl IntoIterator<Item = &'a str>,
        flags: impl IntoIterator<Item = &'a str>,
    ) -> Self {
        Self {
            names: Names {
                contexts: contexts.into_iter().map(str::to_owned).collect(),
                flags: flags.into_iter().map(str::to_owned).collect(),
            },
        }
    }

    /// Whether `name` is a known context, flag or value key.
    pub fn contains(&self, name: &str) -> bool {
        self.names.contexts.contains(name) || self.names.flags.contains(name)
    }

    /// The known view contexts, sorted.
    pub fn contexts(&self) -> impl Iterator<Item = &str> {
        self.names.contexts.iter().map(String::as_str)
    }

    /// The known flags and value keys, sorted.
    pub fn flags(&self) -> impl Iterator<Item = &str> {
        self.names.flags.iter().map(String::as_str)
    }

    /// Whether `name` is a known flag or value key (not a view context).
    pub fn is_flag(&self, name: &str) -> bool {
        self.names.flags.contains(name) && !self.names.contexts.contains(name)
    }

    /// Every known name (contexts, then flags), for "did you mean".
    pub fn all(&self) -> impl Iterator<Item = &str> {
        self.names
            .contexts
            .iter()
            .chain(&self.names.flags)
            .map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_standard_names_are_known_and_views_can_add_theirs() {
        let known = KnownNames::current();
        assert!(known.contains("ResourceTable") && known.contains("Editing"));
        assert!(!known.contains("KnownNamesTestView"));
        register_context("KnownNamesTestView");
        assert!(KnownNames::current().contains("KnownNamesTestView"));
        assert!(!known.contains("KnownNamesTestView"), "a snapshot is fixed");
    }
}
