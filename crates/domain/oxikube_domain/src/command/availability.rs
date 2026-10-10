//! Where a command can run: [`ViewContext`], [`SelectionKind`] and [`Availability`].
//!
//! Availability is *data*, not a closure, so it lives in the static command registry, can be
//! listed by the palette and the help overlay, and can be shown to an agent. It is evaluated
//! by `oxikube_app::command_bus` against a `CommandContext` (the focused view, the session's
//! read-only flag and capabilities, the selection).
//!
//! "Available" means *runnable here*, not *allowed by the guard*: the guard (read-only mode,
//! confirmation tier, audit) still decides at dispatch. A command that
//! [`requires_writable`](Availability::requires_writable) is therefore **hidden** from the
//! palette's default list on a read-only session, so the palette never offers what is certain
//! to be refused; the "show all" list keeps it and says why it cannot run.

use std::fmt;

use serde::Serialize;

/// The kind of view that has the user's focus, as far as commands are concerned.
///
/// Each variant maps to a keymap key context ([`key_context`](Self::key_context)), so a binding
/// scoped to `LogView` and a command available in [`ViewContext::Logs`] agree on where they
/// apply (`oxikube_keymap` tests the correspondence).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewContext {
    /// The workspace itself (no view in focus): cluster tabs, docks, the palette.
    Workspace,
    /// The cluster catalog home.
    Catalog,
    /// A resource table.
    Table,
    /// The resource detail drawer or its pinned tab.
    Detail,
    /// The log viewer.
    Logs,
    /// A terminal.
    Terminal,
    /// The manifest editor.
    Editor,
    /// The command palette and its pickers.
    Palette,
}

impl ViewContext {
    /// Every view context.
    pub const ALL: [ViewContext; 8] = [
        ViewContext::Workspace,
        ViewContext::Catalog,
        ViewContext::Table,
        ViewContext::Detail,
        ViewContext::Logs,
        ViewContext::Terminal,
        ViewContext::Editor,
        ViewContext::Palette,
    ];

    /// The keymap key context name of this view (`LogView`, `ResourceTable`, ...).
    pub const fn key_context(self) -> &'static str {
        match self {
            ViewContext::Workspace => "Workspace",
            ViewContext::Catalog => "Catalog",
            ViewContext::Table => "ResourceTable",
            ViewContext::Detail => "DetailDrawer",
            ViewContext::Logs => "LogView",
            ViewContext::Terminal => "Terminal",
            ViewContext::Editor => "ManifestEditor",
            ViewContext::Palette => "Palette",
        }
    }
}

impl fmt::Display for ViewContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.key_context())
    }
}

/// What a command needs selected before it can run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectionKind {
    /// No selection needed (the default).
    None,
    /// Exactly one object selected.
    One,
    /// One or more objects selected (a bulk-capable action).
    Many,
    /// One or more objects of one kind selected. The version is not compared: a kind keeps
    /// its identity across served versions.
    OfKind {
        /// API group; empty for the core group.
        group: &'static str,
        /// Kind, for example `Pod`.
        kind: &'static str,
    },
}

impl SelectionKind {
    /// One or more core-group objects of `kind` (`Pod`, `Node`).
    pub const fn core(kind: &'static str) -> Self {
        SelectionKind::OfKind { group: "", kind }
    }

    /// Whether the command needs any selection at all.
    pub const fn needs_selection(self) -> bool {
        !matches!(self, SelectionKind::None)
    }
}

/// Where and when a command can run. See the module docs of `availability`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Availability {
    /// The views the command runs in; empty means every view.
    pub views: &'static [ViewContext],
    /// What must be selected.
    pub selection: SelectionKind,
    /// The command changes cluster state, so it is unavailable while the session is read-only.
    /// Set by [`CommandMeta::mutation`](super::CommandMeta::mutation); the guard enforces the
    /// same rule at dispatch.
    pub requires_writable: bool,
}

impl Availability {
    /// Every view, no selection, no write access needed.
    pub const EVERYWHERE: Availability = Availability {
        views: &[],
        selection: SelectionKind::None,
        requires_writable: false,
    };

    /// The same availability limited to `views`.
    #[must_use]
    pub const fn in_views(mut self, views: &'static [ViewContext]) -> Self {
        self.views = views;
        self
    }

    /// The same availability with a selection requirement.
    #[must_use]
    pub const fn selecting(mut self, selection: SelectionKind) -> Self {
        self.selection = selection;
        self
    }

    /// The same availability, unavailable on a read-only session.
    #[must_use]
    pub const fn writable(mut self) -> Self {
        self.requires_writable = true;
        self
    }

    /// Whether the command runs in `view`.
    pub fn in_view(&self, view: ViewContext) -> bool {
        self.views.is_empty() || self.views.contains(&view)
    }
}

impl Default for Availability {
    fn default() -> Self {
        Self::EVERYWHERE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{COMMANDS, CommandCategory, CommandId, CommandScope};

    #[test]
    fn empty_views_mean_every_view() {
        for view in ViewContext::ALL {
            assert!(Availability::EVERYWHERE.in_view(view));
        }
        let logs = Availability::EVERYWHERE.in_views(&[ViewContext::Logs]);
        assert!(logs.in_view(ViewContext::Logs));
        assert!(!logs.in_view(ViewContext::Table));
    }

    #[test]
    fn key_context_names_are_unique() {
        let mut names: Vec<_> = ViewContext::ALL.iter().map(|v| v.key_context()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), ViewContext::ALL.len());
    }

    #[test]
    fn builders_compose() {
        let a = Availability::EVERYWHERE
            .in_views(&[ViewContext::Table])
            .selecting(SelectionKind::core("Pod"))
            .writable();
        assert_eq!(a.views, &[ViewContext::Table]);
        assert!(a.requires_writable);
        assert!(a.selection.needs_selection());
        assert!(!SelectionKind::None.needs_selection());
    }

    #[test]
    fn every_mutation_requires_a_writable_session_and_nothing_else_does() {
        for meta in COMMANDS {
            assert_eq!(
                meta.availability.requires_writable, meta.mutating,
                "{}: requires_writable must mirror `mutating`",
                meta.id
            );
        }
    }

    #[test]
    fn every_declared_command_has_a_named_category() {
        for meta in COMMANDS {
            assert_ne!(
                meta.category,
                CommandCategory::Other,
                "{}: add its namespace to CommandCategory::of",
                meta.id
            );
            assert_eq!(meta.category, CommandCategory::of(meta.id));
        }
    }

    #[test]
    fn the_keymap_action_is_the_command_id() {
        for meta in COMMANDS {
            assert_eq!(meta.keymap_action(), meta.id.as_str());
            assert!(crate::command::lookup_str(meta.keymap_action()).is_some());
        }
    }

    #[test]
    fn selection_scoped_availability_is_declared_for_object_actions() {
        // A command that acts on "the selected object" names the selection it needs, so the
        // palette can hide it when nothing is selected.
        for id in [
            CommandId::POD_DELETE,
            CommandId::POD_SHELL,
            CommandId::NODE_CORDON,
            CommandId::RESOURCE_DELETE,
            CommandId::WORKLOAD_SCALE,
            CommandId::RESOURCE_VIEW_YAML,
        ] {
            let meta = crate::command::lookup(id).unwrap();
            assert!(meta.availability.selection.needs_selection(), "{id}");
            assert!(!meta.availability.views.is_empty(), "{id}");
        }
        for meta in COMMANDS {
            if meta.scope == CommandScope::Global {
                assert!(
                    !meta.availability.selection.needs_selection(),
                    "{}",
                    meta.id
                );
            }
        }
    }

    #[test]
    fn view_bound_namespaces_stay_in_their_view() {
        for meta in COMMANDS {
            let id = meta.id;
            if id.namespace() == "logs" {
                assert_eq!(meta.availability.views, &[ViewContext::Logs], "{id}");
            }
            if id.namespace() == "terminal" && id != CommandId::TERMINAL_NEW {
                assert_eq!(meta.availability.views, &[ViewContext::Terminal], "{id}");
            }
        }
    }
}
