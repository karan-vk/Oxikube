//! `terminal::Copy` and `terminal::Paste` on the command bus.
//!
//! The keymap reaches the focused terminal through GPUI actions of the same names; the palette,
//! context menus and agents (`app.terminal_copy`, `app.terminal_paste`) go through the bus. The
//! handlers queue a [`TerminalInputCommand`] on a [`TerminalInputSink`]; the window drains it on
//! the UI thread and calls [`run`], which dispatches the action to whatever has focus, so the
//! focused terminal's element does the work (and the paste dialog, if one is due, appears in
//! that window). Nothing here reads or changes a cluster: no `MutationGuard` tier.

use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use gpui::{App, Window};
use oxikube_app::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};
use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::{OxiError, OxiResult};

use super::{Copy, Paste};

/// What the window should do to its focused terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalInputCommand {
    /// Copy the selection.
    Copy,
    /// Paste the clipboard.
    Paste,
}

/// A handle on a window's terminal-input queue. Cheap to clone; usable from any thread.
#[derive(Clone, Debug)]
pub struct TerminalInputSink {
    tx: UnboundedSender<TerminalInputCommand>,
}

impl TerminalInputSink {
    /// A sink and the receiver the window drains on the UI thread (calling [`run`]).
    pub fn channel() -> (Self, UnboundedReceiver<TerminalInputCommand>) {
        let (tx, rx) = unbounded();
        (Self { tx }, rx)
    }

    fn send(&self, command: TerminalInputCommand) -> OxiResult<()> {
        self.tx
            .unbounded_send(command)
            .map_err(|_| OxiError::internal("the window with the terminal is gone"))
    }
}

/// Registers `terminal::Copy` and `terminal::Paste` on `registry`. Call it from the binary's
/// command setup: `registry.install("oxikube_terminal", |r| register_input_commands(r, sink))`.
///
/// # Errors
///
/// A [`RegisterError`] when an id is registered twice (a wiring bug).
pub fn register_input_commands(
    registry: &mut CommandRegistry,
    sink: TerminalInputSink,
) -> Result<(), RegisterError> {
    for id in [CommandId::TERMINAL_COPY, CommandId::TERMINAL_PASTE] {
        let meta = *command::lookup(id).ok_or(RegisterError::Undeclared(id))?;
        let sink = sink.clone();
        registry.register(meta, move |command: Command, _: HandlerContext| {
            let sink = sink.clone();
            async move {
                match command {
                    Command::TerminalCopy => sink.send(TerminalInputCommand::Copy)?,
                    Command::TerminalPaste => sink.send(TerminalInputCommand::Paste)?,
                    _ => return Err(OxiError::validation("not a terminal input command")),
                }
                Ok(CommandOutput::none())
            }
        })?;
    }
    Ok(())
}

/// Runs `command` on the UI thread: dispatches the matching action to the window's focused
/// element. Deferred one turn so a palette that was just dismissed has handed focus back to the
/// terminal first. Does nothing when no terminal is focused.
pub fn run(command: TerminalInputCommand, window: &mut Window, cx: &mut App) {
    window.defer(cx, move |window, cx| match command {
        TerminalInputCommand::Copy => window.dispatch_action(Box::new(Copy), cx),
        TerminalInputCommand::Paste => window.dispatch_action(Box::new(Paste), cx),
    });
}
