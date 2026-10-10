//! The tab commands: the queue the key handlers and the `CommandBus` push into, and what each
//! command does.
//!
//! `cluster::Select`, `cluster::SwitchTab`, `cluster::NextTab`, `cluster::PreviousTab` and
//! `cluster::CloseTab` change what the window shows, never a cluster, so none goes through
//! `MutationGuard`. Each is declared in `oxikube_domain::command` (so it has an MCP tool stub,
//! `app.cluster_select` and so on) and registered on the bus by [`register_commands`], whose
//! handlers push the command into the controller's queue; the keys (`cmd-1..9`, `ctrl-tab`),
//! the hotbar and the tab's close button reach the same `apply`.
//!
//! They are immediate commands (E05-P600): when the UI runs one (`ClusterCommandRunner`, the
//! keys), the queue is applied in that same update ([`apply_queued`]), so the frame drawn right
//! after the input shows the other tab. A command queued from another thread (an agent) is
//! applied by the controller's task when it wakes.

use gpui::{AnyWindowHandle, App, Context, Window};
use oxikube_app::command_bus::{
    CommandOutput, CommandRegistry, HandlerContext, Immediate, RegisterError,
};
use oxikube_domain::OxiError;
use oxikube_domain::command::{self, Command};

use super::{ClusterTabs, TabsWindows};
use crate::cluster_tab::dispatch::{CommandSink, TAB_COMMANDS};

impl ClusterTabs {
    /// Runs a tab command. Returns whether `command` is one of the tab commands and found
    /// something to act on (a cluster with a tab, an index within the open tabs).
    pub fn apply(
        &mut self,
        command: &Command,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        match command {
            Command::ClusterSelect { cluster } => self.activate(cluster, window, cx),
            Command::ClusterSwitchTab { index } => match usize::from(*index).checked_sub(1) {
                Some(index) => self.switch_to_index(index, window, cx),
                None => false,
            },
            Command::ClusterNextTab => self.next(window, cx),
            Command::ClusterPreviousTab => self.previous(window, cx),
            Command::ClusterCloseTab { cluster } => {
                let known = self.tabs.contains_key(cluster);
                self.request_close(cluster, window, cx);
                known
            }
            other => {
                tracing::warn!(command = %other.id(), "not a cluster tab command");
                false
            }
        }
    }

    /// Applies every command queued and not applied yet, now. The receiver is borrowed only to
    /// take one command at a time (applying one may queue another), and `try_recv` leaves the
    /// waker of the controller's task in place.
    pub(crate) fn apply_queued(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        loop {
            let next = self.queue.borrow_mut().try_recv();
            let Ok(command) = next else { break };
            self.apply(&command, window, cx);
        }
    }
}

/// Applies the tab commands queued for `window`'s controller before the current update ends, for
/// callers that cannot borrow the window (a key handler while the window dispatches the key, a
/// view inside its own update): the deferred callback runs when that update returns, still before
/// the next frame.
pub(crate) fn apply_before_next_frame(window: AnyWindowHandle, cx: &mut App) {
    cx.defer(move |cx| {
        window
            .update(cx, |_, window, cx| apply_queued(window, cx))
            .ok();
    });
}

/// Applies the tab commands queued for `window`'s controller now, inside the caller's update
/// (the window must not be borrowed elsewhere). Does nothing in a window without cluster tabs.
pub(crate) fn apply_queued(window: &mut Window, cx: &mut App) {
    let id = window.window_handle().window_id();
    if let Some(tabs) = TabsWindows::controller(cx, id).and_then(|tabs| tabs.upgrade()) {
        tabs.update(cx, |tabs, cx| tabs.apply_queued(window, cx));
    }
}

/// Registers the tab commands on `registry` as immediate commands (MCP tool stubs included), with
/// handlers that push the command into `sink` ([`ClusterTabs::command_sink`]); the runner then
/// applies the queue in the same update. Call it from the binary's command
/// setup: `registry.install("oxikube_workspace", |r| register_commands(r, sink))`.
///
/// # Errors
///
/// The first [`RegisterError`] (a duplicate registration is a bug in the binary's setup).
pub fn register_commands(
    registry: &mut CommandRegistry,
    sink: CommandSink,
) -> Result<(), RegisterError> {
    for id in TAB_COMMANDS {
        let meta = *command::lookup(id).ok_or(RegisterError::Undeclared(id))?;
        let sink = sink.clone();
        registry.register_immediate(meta, move |command: Command, _: HandlerContext| {
            if sink.send(command) {
                Ok(Immediate::done(CommandOutput::none()))
            } else {
                Err(OxiError::internal(
                    "the window with the cluster tabs is gone",
                ))
            }
        })?;
    }
    Ok(())
}
