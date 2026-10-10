//! Where UI code sends [`Command`]s: [`CommandDispatcher`], and the pieces that route the
//! cluster-tab commands to the [`ClusterTabs`](super::ClusterTabs) controller.
//!
//! - [`CommandDispatcher`]: the trait every view sends its commands through (the catalog, the
//!   hotbar, the tabs). The palette, the keymap and the buttons all end here, so each action
//!   has one behaviour (non-negotiable 4).
//! - [`CommandSink`]: a `Send + Sync` handle on the controller's command queue. A `CommandBus`
//!   handler (any thread) pushes a command; the controller applies it on the UI thread, with the
//!   window it needs. This is the bridge from the bus to the tab UI.
//! - [`TabsDispatcher`]: a dispatcher that sends the tab commands (`cluster::Select`,
//!   `cluster::SwitchTab`, `cluster::NextTab`, `cluster::PreviousTab`, `cluster::CloseTab`) to
//!   the controller, applied before the next frame, and hands every other command to the
//!   dispatcher it wraps (the catalog's service dispatcher, in the app).

use std::rc::Rc;

use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use gpui::App;
use oxikube_domain::command::{Command, CommandId};

/// Where a view sends a [`Command`]. The palette, the keymap and the view's own buttons all end
/// here, so each runs one behaviour (non-negotiable 4).
///
/// The production implementation hands the command to the `CommandBus` (E06-S02); until that is
/// wired into the binary, the catalog's `ServiceDispatcher` runs the cluster commands and
/// [`TabsDispatcher`] the tab commands. Tests record what was sent.
///
/// `dispatch` returns at once: the work runs off the UI thread and its outcome reaches views
/// through the session update stream, never through a return value.
pub trait CommandDispatcher: 'static {
    /// Sends `command`.
    fn dispatch(&self, command: Command, cx: &mut App);
}

/// The commands the cluster tabs run themselves.
pub(super) const TAB_COMMANDS: [CommandId; 5] = [
    CommandId::CLUSTER_SELECT,
    CommandId::CLUSTER_SWITCH_TAB,
    CommandId::CLUSTER_NEXT_TAB,
    CommandId::CLUSTER_PREVIOUS_TAB,
    CommandId::CLUSTER_CLOSE_TAB,
];

/// Whether `command` is one the cluster tabs run themselves.
pub fn is_tab_command(command: &Command) -> bool {
    TAB_COMMANDS.contains(&command.id())
}

/// A handle on the controller's command queue. Cheap to clone; usable from any thread.
#[derive(Clone, Debug)]
pub struct CommandSink {
    tx: UnboundedSender<Command>,
}

impl CommandSink {
    pub(crate) fn channel() -> (Self, UnboundedReceiver<Command>) {
        let (tx, rx) = unbounded();
        (Self { tx }, rx)
    }

    /// Queues `command` for the controller. Returns `false` when the controller is gone (its
    /// window closed), in which case nothing runs.
    pub fn send(&self, command: Command) -> bool {
        self.tx.unbounded_send(command).is_ok()
    }
}

/// Sends the tab commands to the controller and everything else to `inner`.
#[derive(Clone)]
pub struct TabsDispatcher {
    inner: Rc<dyn CommandDispatcher>,
    sink: CommandSink,
}

impl TabsDispatcher {
    /// A dispatcher over `inner` for the commands the tabs do not run, and `sink` for the ones
    /// they do ([`ClusterTabs::command_sink`](super::ClusterTabs::command_sink)).
    pub fn new(inner: Rc<dyn CommandDispatcher>, sink: CommandSink) -> Self {
        Self { inner, sink }
    }
}

impl CommandDispatcher for TabsDispatcher {
    fn dispatch(&self, command: Command, cx: &mut App) {
        if is_tab_command(&command) {
            if !self.sink.send(command) {
                tracing::debug!("the cluster tabs are gone: command dropped");
            } else if let Some(window) = cx.active_window() {
                // A click on the hotbar: shown in the frame after it, as the keys are (E05-P600).
                super::apply_before_next_frame(window, cx);
            }
        } else {
            self.inner.dispatch(command, cx);
        }
    }
}
