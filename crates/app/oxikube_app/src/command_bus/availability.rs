//! [`CommandContext`]: what is true right now, and `check`: can a command run in it.
//!
//! The palette, the help overlay, context menus and hosted agents all ask the same question,
//! "which commands can run here?", of the same data: a command's
//! [`Availability`](oxikube_domain::command::Availability) (plain data in the domain) against
//! the [`CommandContext`] the caller builds from the focused view and the session. Nothing here
//! is a closure over UI types, so the answer is the same on every surface and testable without
//! a window.
//!
//! "Available" is not "allowed by the guard": the guard still enforces read-only mode,
//! confirmation and audit at dispatch. This layer only avoids offering what is certain to be
//! refused (a mutation on a read-only session, a command of another view).

use std::fmt;

use oxikube_domain::Capabilities;
use oxikube_domain::command::{CommandMeta, CommandScope, SelectionKind, ViewContext};
use oxikube_domain::ids::Gvk;

use crate::actions::ActionContext;

/// What is selected in the focused view.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Selection {
    count: usize,
    kind: Option<Gvk>,
}

impl Selection {
    /// Nothing selected.
    pub fn none() -> Self {
        Self::default()
    }

    /// One object of `kind`.
    pub fn one(kind: Gvk) -> Self {
        Self {
            count: 1,
            kind: Some(kind),
        }
    }

    /// `count` objects of one `kind`.
    pub fn many(kind: Gvk, count: usize) -> Self {
        Self {
            count,
            kind: Some(kind),
        }
    }

    /// `count` objects of different kinds.
    pub fn mixed(count: usize) -> Self {
        Self { count, kind: None }
    }

    /// How many objects are selected.
    pub fn count(&self) -> usize {
        self.count
    }

    /// The kind of the selected objects, when they are all of one kind.
    pub fn kind(&self) -> Option<&Gvk> {
        self.kind.as_ref()
    }

    /// Whether the selection satisfies `needs`.
    pub fn satisfies(&self, needs: SelectionKind) -> bool {
        match needs {
            SelectionKind::None => true,
            SelectionKind::One => self.count == 1,
            SelectionKind::Many => self.count >= 1,
            SelectionKind::OfKind { group, kind } => {
                self.count >= 1
                    && self
                        .kind
                        .as_ref()
                        .is_some_and(|k| &*k.group == group && &*k.kind == kind)
            }
        }
    }
}

/// Everything availability depends on. Cheap to build per keystroke or per focus change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandContext {
    /// The focused view.
    pub view: ViewContext,
    /// Whether a cluster is active (a connected tab is shown). Commands scoped to a cluster,
    /// namespace, kind or selection need one.
    pub cluster_active: bool,
    /// What the active session can do.
    pub capabilities: Capabilities,
    /// Whether the active session is read-only.
    pub read_only: bool,
    /// Whether a shell, attach or exec still opens on a read-only session
    /// (`exec_in_read_only`).
    pub exec_in_read_only: bool,
    /// What is selected in the focused view.
    pub selection: Selection,
}

impl CommandContext {
    /// A context with no cluster: the catalog or an empty workspace.
    pub fn new(view: ViewContext) -> Self {
        Self {
            view,
            cluster_active: false,
            capabilities: Capabilities::empty(),
            read_only: false,
            exec_in_read_only: false,
            selection: Selection::none(),
        }
    }

    /// A context in an active session described by `session`.
    pub fn in_session(view: ViewContext, session: &ActionContext) -> Self {
        Self {
            view,
            cluster_active: true,
            capabilities: session.capabilities,
            read_only: session.read_only,
            exec_in_read_only: session.exec_in_read_only,
            selection: Selection::none(),
        }
    }

    /// The same context with `selection`.
    #[must_use]
    pub fn selecting(mut self, selection: Selection) -> Self {
        self.selection = selection;
        self
    }

    /// The same context with the read-only flag set.
    #[must_use]
    pub fn read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }

    /// The same context with `capabilities`.
    #[must_use]
    pub fn with_capabilities(mut self, capabilities: Capabilities) -> Self {
        self.capabilities = capabilities;
        self
    }
}

/// Why a command cannot run in a [`CommandContext`] (the first reason found).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unavailable {
    /// The command acts on a cluster and none is active.
    NoCluster,
    /// The command belongs to other views than the focused one.
    OtherView,
    /// The session lacks capabilities the command needs.
    Capabilities(Capabilities),
    /// The session is read-only and the command changes the cluster.
    ReadOnly,
    /// The session is read-only and shells are blocked there (`exec_in_read_only`).
    ExecReadOnly,
    /// The command needs a selection that is missing or of another size or kind.
    Selection(SelectionKind),
}

impl fmt::Display for Unavailable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Unavailable::NoCluster => f.write_str("Connect to a cluster first"),
            Unavailable::OtherView => f.write_str("Not available in this view"),
            Unavailable::Capabilities(missing) => {
                write!(
                    f,
                    "Needs {}",
                    missing.names().collect::<Vec<_>>().join(", ")
                )
            }
            Unavailable::ReadOnly => f.write_str("This cluster is read-only"),
            Unavailable::ExecReadOnly => f.write_str("Shells are blocked on a read-only cluster"),
            Unavailable::Selection(SelectionKind::One) => f.write_str("Select one object"),
            Unavailable::Selection(SelectionKind::Many) => f.write_str("Select an object"),
            Unavailable::Selection(SelectionKind::OfKind { kind, .. }) => {
                write!(f, "Select a {kind}")
            }
            Unavailable::Selection(SelectionKind::None) => f.write_str("Clear the selection"),
        }
    }
}

/// Whether `meta` can run in `ctx`; the first failing requirement otherwise.
///
/// The checks run from the cheapest and most telling to the most specific: cluster, view,
/// capabilities, read-only, selection.
pub(super) fn check(meta: &CommandMeta, ctx: &CommandContext) -> Result<(), Unavailable> {
    if meta.scope != CommandScope::Global && !ctx.cluster_active {
        return Err(Unavailable::NoCluster);
    }
    let availability = &meta.availability;
    if !availability.in_view(ctx.view) {
        return Err(Unavailable::OtherView);
    }
    let missing = meta.needs.missing_from(ctx.capabilities);
    if !missing.is_empty() {
        return Err(Unavailable::Capabilities(missing));
    }
    if ctx.read_only {
        if availability.requires_writable {
            return Err(Unavailable::ReadOnly);
        }
        if meta.exec && !ctx.exec_in_read_only {
            return Err(Unavailable::ExecReadOnly);
        }
    }
    if !ctx.selection.satisfies(availability.selection) {
        return Err(Unavailable::Selection(availability.selection));
    }
    Ok(())
}
