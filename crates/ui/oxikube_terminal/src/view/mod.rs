//! The terminal in the workspace (E09-S07): [`TerminalView`], a workspace `Item` that moves
//! between the panes and the bottom dock, the cluster's [`TerminalPanel`] in its bottom dock, and
//! the `terminal::New` / `terminal::Split` / `terminal::Close` commands.
//!
//! | Module | What |
//! |---|---|
//! | `descriptor` | [`BackendDescriptor`]: what a terminal runs (kind, program, directory, cluster or pod); the tab's whole saved state |
//! | `launch` | [`TerminalLauncher`]: starts the process a descriptor describes, off the UI thread; [`LocalLauncher`], the app's (local shells with the cluster environment) |
//! | `services` | [`TerminalServices`]: launcher, command dispatcher, paste confirmation; an app global so the layout restore builds terminals too |
//! | `terminal_view` | [`TerminalView`]: owns the [`TerminalState`](crate::TerminalState) (grid, backend, tasks) for the tab's life; title from the process |
//! | `render` | the element, the starting line, the banner and the dimming of a session that cannot take input |
//! | `lifecycle` | [`Lifecycle`]: connecting / running / disconnected / exited / failed / closed as a small enum, the error taxonomy ([`Failure`]) and the [`Banner`] text (E09-S12) |
//! | `recover`, `strip` | Reconnect (pod) and Restart (local shell) from the same descriptor, and the banner strip with its buttons |
//! | `item` | the `Item` impl: tab title = process or pod name, dirty = a process runs, icon by kind, cluster mark, dockable, split = a fresh copy, close ends the process, saved = the descriptor only |
//! | `panel` | [`TerminalPanel`]: the bottom-dock panel of a cluster's terminals, and [`ensure_terminal_panel`] |
//! | `commands` | the bus handlers: [`register_view_commands`] queues a [`TerminalRequest`] on the window's [`TerminalViewSink`] |
//! | `host`, `controller` | [`TerminalViews`]: applies the requests in the shown workspace, through a [`TerminalHost`] ([`ClusterTerminalHost`] in the app) |
//!
//! One `TerminalView` per process: moving its tab between panes and the dock moves the entity, so
//! the backend and the grid are never recreated. Closing the tab ends the process and drops the
//! tasks (abort on drop). A restored tab starts a fresh process from its descriptor; the
//! scrollback is never saved (non-negotiable 5).

mod commands;
mod controller;
mod descriptor;
mod host;
mod item;
mod launch;
pub mod lifecycle;
mod panel;
mod recover;
mod render;
mod services;
mod strip;
mod terminal_view;

use gpui::{App, actions};
use oxikube_domain::command::Command;

pub use commands::{TerminalRequest, TerminalViewSink, register_view_commands};
pub use controller::{TerminalViews, TerminalViewsDeps};
pub use descriptor::BackendDescriptor;
pub use host::{ClusterTerminalHost, TerminalHost};
pub use item::TERMINAL_ITEM_KIND;
pub use launch::{Launch, LocalLauncher, TerminalLauncher};
pub use lifecycle::{Banner, BannerAction, Failure, FailureKind, Lifecycle, Tone};
pub use panel::{TerminalPanel, ensure_terminal_panel};
pub use services::TerminalServices;
pub use terminal_view::TerminalView;

actions!(
    terminal,
    [
        /// Open a local shell in a new terminal (`terminal::New` on the bus).
        New,
        /// Open a new terminal in a pane beside the active one (`terminal::Split`).
        Split,
        /// Close the focused terminal (`terminal::Close`).
        Close,
    ]
);

actions!(
    terminal_panel,
    [
        /// Show and focus the terminal panel of the shown cluster, or hide it (the panel's toggle
        /// action; the workspace that holds the panel handles it).
        TogglePanel,
    ]
);

/// Registers the terminal tab: the builder that rebuilds saved terminal tabs and the actions that
/// send `terminal::New` / `Split` / `Close` to the bus (through the installed services'
/// dispatcher). Called by [`crate::init`].
pub(crate) fn init(cx: &mut App) {
    item::register_builder(cx);
    cx.on_action(|_: &New, cx| dispatch(Command::TerminalNew { cluster: None }, cx));
    cx.on_action(|_: &Split, cx| dispatch(Command::TerminalSplit, cx));
    cx.on_action(|_: &Close, cx| dispatch(Command::TerminalClose, cx));
}

/// Makes `services` the app's: terminals restored with a layout, and the keymap's
/// `terminal::*` actions, use them. Call it when the window mounts (the binary), before the
/// layout restore.
pub fn install(services: TerminalServices, cx: &mut App) {
    cx.set_global(services);
}

fn dispatch(command: Command, cx: &mut App) {
    if let Some(services) = TerminalServices::try_global(cx) {
        services.dispatch(command, cx);
    }
}
