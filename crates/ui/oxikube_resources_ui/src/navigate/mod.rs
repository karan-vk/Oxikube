//! Opening a kind's list (E07-S11): the navigation `Command` and the registry of views that
//! answer it.
//!
//! Tiles, sidebar entries and (later) the palette and `:` jump all send
//! `resource::OpenList { cluster, gvk }` through the `CommandBus`. It reads, never mutates, so it
//! needs no `MutationGuard`; its MCP tool stub (`k8s.resource_open_list`) is declared with the
//! command in `oxikube_domain`.
//!
//! The command's handler ([`register_commands`]) only hands the request to the UI thread; what
//! opens is up to the views that registered with [`KindViews`] (the generic resource table of
//! E07-S03 registers itself there). When none claims the kind, [`open_kind`] says so and the
//! caller shows a notice instead of doing nothing.

use std::rc::Rc;

use futures::channel::mpsc::UnboundedSender;
use gpui::{App, Entity, Global, Window};
use oxikube_app::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};
use oxikube_domain::OxiError;
use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::ids::{ClusterId, Gvk};
use oxikube_workspace::Workspace;

/// A request to open the list of `gvk` in `cluster`'s tab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenKind {
    /// The cluster whose tab shows the list.
    pub cluster: ClusterId,
    /// The kind to list.
    pub gvk: Gvk,
}

/// Opens a kind's list in a cluster's workspace; returns whether this opener handled the kind.
pub type KindOpener = dyn Fn(&OpenKind, &Entity<Workspace>, &mut Window, &mut App) -> bool;

/// The views that can show a kind's list, asked in registration order. A GPUI global.
#[derive(Default)]
pub struct KindViews {
    openers: Vec<Rc<KindOpener>>,
}

impl Global for KindViews {}

impl KindViews {
    /// Adds an opener. Later registrations are asked after earlier ones.
    pub fn register(
        cx: &mut App,
        opener: impl Fn(&OpenKind, &Entity<Workspace>, &mut Window, &mut App) -> bool + 'static,
    ) {
        cx.default_global::<KindViews>()
            .openers
            .push(Rc::new(opener));
    }
}

/// Opens the list `request` names in `workspace` (the cluster's tab) through the first registered
/// opener that takes it. `false` when nothing is registered for the kind (yet).
pub fn open_kind(
    request: &OpenKind,
    workspace: &Entity<Workspace>,
    window: &mut Window,
    cx: &mut App,
) -> bool {
    let openers: Vec<Rc<KindOpener>> = cx
        .try_global::<KindViews>()
        .map(|views| views.openers.clone())
        .unwrap_or_default();
    openers
        .iter()
        .any(|open| open(request, workspace, window, cx))
}

/// Registers the handler of `resource::OpenList`: it checks the command and sends the request
/// to `requests` (the UI thread applies it with [`open_kind`]).
///
/// # Errors
///
/// [`RegisterError`] when the id is already registered or undeclared.
pub fn register_commands(
    registry: &mut CommandRegistry,
    requests: UnboundedSender<OpenKind>,
) -> Result<(), RegisterError> {
    let meta = command::lookup(CommandId::RESOURCE_OPEN_LIST)
        .copied()
        .ok_or(RegisterError::Undeclared(CommandId::RESOURCE_OPEN_LIST))?;
    registry.register(meta, move |command: Command, _: HandlerContext| {
        let requests = requests.clone();
        async move {
            let Command::ResourceOpenList { cluster, gvk } = command else {
                return Err(OxiError::validation("not a resource::OpenList command"));
            };
            requests
                .unbounded_send(OpenKind { cluster, gvk })
                .map_err(|_| OxiError::internal("the main window is gone"))?;
            Ok(CommandOutput::none())
        }
    })
}

#[cfg(test)]
mod tests;
