//! `terminal::New`, `terminal::Split`, `terminal::Close`, `terminal::Reconnect` and
//! `terminal::Restart` on the command bus.
//!
//! The keymap (through the GPUI actions of the same names), the terminal panel's button, the
//! palette and agents (`app.terminal_new`, ...) all dispatch the bus command. Its handler queues
//! a [`TerminalRequest`] on the window's [`TerminalViewSink`]; the window's
//! [`TerminalViews`](super::TerminalViews) applies it on the UI thread. Nothing here reads or
//! changes a cluster: no `MutationGuard` tier (a cluster shell's commands are the user's own).

use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use oxikube_app::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};
use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::ids::ClusterId;
use oxikube_domain::{OxiError, OxiResult};

use super::BackendDescriptor;

/// What the window's terminal views should do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalRequest {
    /// `terminal::New`: a local shell for `cluster` (the displayed cluster when `None`).
    New {
        /// The cluster whose tab gets the terminal.
        cluster: Option<ClusterId>,
    },
    /// A terminal running `descriptor` in the bottom dock of its cluster's tab (the window's
    /// own workspace without a cluster): what another view asks for when it wants a process in a
    /// terminal, such as the log viewer's "Tail in terminal" (`logs::TailInTerminal`). Not a
    /// command of its own: the asking view has the command.
    Open {
        /// What to run.
        descriptor: BackendDescriptor,
    },
    /// `terminal::Split`: a new terminal in a pane beside the active one.
    Split,
    /// `terminal::Close`: close the focused terminal.
    Close,
    /// `terminal::Reconnect`: open the focused pod terminal's session again (E09-S12).
    Reconnect,
    /// `terminal::Restart`: start a fresh shell in the focused local terminal (E09-S12).
    Restart,
}

/// A handle on a window's terminal request queue. Cheap to clone; usable from any thread.
#[derive(Clone, Debug)]
pub struct TerminalViewSink {
    tx: UnboundedSender<TerminalRequest>,
}

impl TerminalViewSink {
    /// A sink and the receiver the window's [`TerminalViews`](super::TerminalViews) drains.
    pub fn channel() -> (Self, UnboundedReceiver<TerminalRequest>) {
        let (tx, rx) = unbounded();
        (Self { tx }, rx)
    }

    /// Asks the window to open a terminal running `descriptor` ([`TerminalRequest::Open`]).
    /// `false` when the window is gone.
    pub fn open(&self, descriptor: BackendDescriptor) -> bool {
        self.send(TerminalRequest::Open { descriptor }).is_ok()
    }

    fn send(&self, request: TerminalRequest) -> OxiResult<()> {
        self.tx
            .unbounded_send(request)
            .map_err(|_| OxiError::internal("the window with the terminals is gone"))
    }
}

/// The request a terminal command asks for; `None` for any other command.
fn request_of(command: Command) -> Option<TerminalRequest> {
    match command {
        Command::TerminalNew { cluster } => Some(TerminalRequest::New { cluster }),
        Command::TerminalSplit => Some(TerminalRequest::Split),
        Command::TerminalClose => Some(TerminalRequest::Close),
        Command::TerminalReconnect => Some(TerminalRequest::Reconnect),
        Command::TerminalRestart => Some(TerminalRequest::Restart),
        _ => None,
    }
}

/// Registers `terminal::New`, `Split`, `Close`, `Reconnect` and `Restart` on `registry` (each with
/// its MCP tool stub). Call it from the binary's command setup:
/// `registry.install("oxikube_terminal", |r| register_view_commands(r, sink))`.
///
/// # Errors
///
/// A [`RegisterError`] when an id is registered twice (a wiring bug).
pub fn register_view_commands(
    registry: &mut CommandRegistry,
    sink: TerminalViewSink,
) -> Result<(), RegisterError> {
    for id in [
        CommandId::TERMINAL_NEW,
        CommandId::TERMINAL_SPLIT,
        CommandId::TERMINAL_CLOSE,
        CommandId::TERMINAL_RECONNECT,
        CommandId::TERMINAL_RESTART,
    ] {
        let meta = *command::lookup(id).ok_or(RegisterError::Undeclared(id))?;
        let sink = sink.clone();
        registry.register(meta, move |command: Command, _: HandlerContext| {
            let sink = sink.clone();
            async move {
                let request = request_of(command)
                    .ok_or_else(|| OxiError::validation("not a terminal view command"))?;
                sink.send(request)?;
                Ok(CommandOutput::none())
            }
        })?;
    }
    Ok(())
}
