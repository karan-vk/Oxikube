//! `terminal::New`, `terminal::Split` and `terminal::Close` on the command bus.
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

/// What the window's terminal views should do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalRequest {
    /// `terminal::New`: a local shell for `cluster` (the displayed cluster when `None`).
    New {
        /// The cluster whose tab gets the terminal.
        cluster: Option<ClusterId>,
    },
    /// `terminal::Split`: a new terminal in a pane beside the active one.
    Split,
    /// `terminal::Close`: close the focused terminal.
    Close,
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

    fn send(&self, request: TerminalRequest) -> OxiResult<()> {
        self.tx
            .unbounded_send(request)
            .map_err(|_| OxiError::internal("the window with the terminals is gone"))
    }
}

/// The request a terminal command asks for; `None` for any other command.
pub fn request_of(command: Command) -> Option<TerminalRequest> {
    match command {
        Command::TerminalNew { cluster } => Some(TerminalRequest::New { cluster }),
        Command::TerminalSplit => Some(TerminalRequest::Split),
        Command::TerminalClose => Some(TerminalRequest::Close),
        _ => None,
    }
}

/// Registers `terminal::New`, `terminal::Split` and `terminal::Close` on `registry` (each with
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
