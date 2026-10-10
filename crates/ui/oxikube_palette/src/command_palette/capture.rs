//! Capturing where the user is when the palette opens: the [`CommandContext`] the commands are
//! filtered by and the [`CommandTarget`] they run on.
//!
//! Taken once, before the palette takes the keyboard focus, from the focused view
//! ([`oxikube_workspace::command_surface`]) and the session ([`PaletteEnv`]). A palette that stays
//! open does not follow the selection: what you open it on is what a command acts on, even
//! after the focus returns to the view.

use gpui::{App, Window};
use oxikube_app::{CommandContext, CommandTarget, Selection};
use oxikube_domain::command::CommandId;
use oxikube_domain::ids::Gvk;
use oxikube_workspace::command_surface::{focused, view_of_focus};

use super::PaletteEnv;

/// Where the palette was opened: what to filter by and what to run on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Captured {
    /// What is true right now: the focused view, the session, the selection.
    pub context: CommandContext,
    /// The cluster, kind and objects a command runs on.
    pub target: CommandTarget,
    /// The commands the focused view runs through its own flow.
    pub own_commands: Vec<CommandId>,
}

/// Reads the focused view and the session. Call it before the palette's own view takes the focus.
pub fn capture(window: &Window, env: &dyn PaletteEnv, cx: &App) -> Captured {
    let (view, mut target, own_commands) = match focused(window, cx) {
        Some(surface) => (surface.view, surface.target, surface.own_commands),
        None => (view_of_focus(window), CommandTarget::none(), Vec::new()),
    };
    if target.cluster.is_none() {
        target.cluster = env.active_cluster(cx);
    }
    let session = target
        .cluster
        .as_ref()
        .and_then(|cluster| env.session(cluster, cx));
    let context = match &session {
        Some(session) => CommandContext::in_session(view, session),
        None => CommandContext::new(view),
    }
    .selecting(selection_of(&target));
    Captured {
        context,
        target,
        own_commands,
    }
}

/// The selection the objects of `target` make: none, or `n` objects of one kind or of several.
pub fn selection_of(target: &CommandTarget) -> Selection {
    let Some(first) = target.targets.first() else {
        return Selection::none();
    };
    let count = target.targets.len();
    let kind: &Gvk = &first.gvk;
    if target.targets.iter().all(|object| &object.gvk == kind) {
        Selection::many(kind.clone(), count)
    } else {
        Selection::mixed(count)
    }
}
