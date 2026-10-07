//! The terminal's own commands on the command bus: `terminal::Copy`, `Paste`, `SelectAll`,
//! `Clear`, `ScrollPageUp` / `PageDown` / `LineUp` / `LineDown`, and `Search` / `SearchNext` /
//! `SearchPrevious` / `SearchClose`.
//!
//! The keymap reaches the focused terminal through GPUI actions of the same names; the palette,
//! context menus and agents (`app.terminal_copy`, ...) go through the bus. The handlers queue a
//! [`TerminalInputCommand`] on a [`TerminalInputSink`]; the window drains it on the UI thread and
//! calls [`run`], which dispatches the action to whatever has focus, so the focused terminal's
//! element (or, for search, its view) does the work (and the paste dialog, if one is due,
//! appears in that window). Nothing here reads or changes a cluster: no `MutationGuard` tier.

use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use gpui::{Action, App, Window};
use oxikube_app::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};
use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::{OxiError, OxiResult};

use super::{
    Clear, Copy, Paste, ScrollLineDown, ScrollLineUp, ScrollPageDown, ScrollPageUp, Search,
    SearchClose, SearchNext, SearchPrevious, SelectAll,
};

/// What the window should do to its focused terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalInputCommand {
    /// Copy the selection.
    Copy,
    /// Paste the clipboard.
    Paste,
    /// Select everything.
    SelectAll,
    /// Clear the scrollback and the screen above the cursor.
    Clear,
    /// Scroll the history one screen up.
    ScrollPageUp,
    /// Scroll the history one screen down.
    ScrollPageDown,
    /// Scroll the history one line up.
    ScrollLineUp,
    /// Scroll the history one line down.
    ScrollLineDown,
    /// Open the search bar.
    Search,
    /// Jump to the next match.
    SearchNext,
    /// Jump to the previous match.
    SearchPrevious,
    /// Close the search bar.
    SearchClose,
}

impl TerminalInputCommand {
    /// The command id this is the request of.
    pub fn id(self) -> CommandId {
        match self {
            Self::Copy => CommandId::TERMINAL_COPY,
            Self::Paste => CommandId::TERMINAL_PASTE,
            Self::SelectAll => CommandId::TERMINAL_SELECT_ALL,
            Self::Clear => CommandId::TERMINAL_CLEAR,
            Self::ScrollPageUp => CommandId::TERMINAL_SCROLL_PAGE_UP,
            Self::ScrollPageDown => CommandId::TERMINAL_SCROLL_PAGE_DOWN,
            Self::ScrollLineUp => CommandId::TERMINAL_SCROLL_LINE_UP,
            Self::ScrollLineDown => CommandId::TERMINAL_SCROLL_LINE_DOWN,
            Self::Search => CommandId::TERMINAL_SEARCH,
            Self::SearchNext => CommandId::TERMINAL_SEARCH_NEXT,
            Self::SearchPrevious => CommandId::TERMINAL_SEARCH_PREVIOUS,
            Self::SearchClose => CommandId::TERMINAL_SEARCH_CLOSE,
        }
    }

    /// Every command of this family.
    pub const ALL: [Self; 12] = [
        Self::Copy,
        Self::Paste,
        Self::SelectAll,
        Self::Clear,
        Self::ScrollPageUp,
        Self::ScrollPageDown,
        Self::ScrollLineUp,
        Self::ScrollLineDown,
        Self::Search,
        Self::SearchNext,
        Self::SearchPrevious,
        Self::SearchClose,
    ];

    /// The request a bus [`Command`] makes; `None` for any other command.
    pub fn of(command: &Command) -> Option<Self> {
        Some(match command {
            Command::TerminalCopy => Self::Copy,
            Command::TerminalPaste => Self::Paste,
            Command::TerminalSelectAll => Self::SelectAll,
            Command::TerminalClear => Self::Clear,
            Command::TerminalScrollPageUp => Self::ScrollPageUp,
            Command::TerminalScrollPageDown => Self::ScrollPageDown,
            Command::TerminalScrollLineUp => Self::ScrollLineUp,
            Command::TerminalScrollLineDown => Self::ScrollLineDown,
            Command::TerminalSearch => Self::Search,
            Command::TerminalSearchNext => Self::SearchNext,
            Command::TerminalSearchPrevious => Self::SearchPrevious,
            Command::TerminalSearchClose => Self::SearchClose,
            _ => return None,
        })
    }

    fn action(self) -> Box<dyn Action> {
        match self {
            Self::Copy => Box::new(Copy),
            Self::Paste => Box::new(Paste),
            Self::SelectAll => Box::new(SelectAll),
            Self::Clear => Box::new(Clear),
            Self::ScrollPageUp => Box::new(ScrollPageUp),
            Self::ScrollPageDown => Box::new(ScrollPageDown),
            Self::ScrollLineUp => Box::new(ScrollLineUp),
            Self::ScrollLineDown => Box::new(ScrollLineDown),
            Self::Search => Box::new(Search),
            Self::SearchNext => Box::new(SearchNext),
            Self::SearchPrevious => Box::new(SearchPrevious),
            Self::SearchClose => Box::new(SearchClose),
        }
    }
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

/// Registers the commands of [`TerminalInputCommand::ALL`] on `registry`, each with its MCP tool
/// stub. Call it from the binary's command setup:
/// `registry.install("oxikube_terminal", |r| register_input_commands(r, sink))`.
///
/// # Errors
///
/// A [`RegisterError`] when an id is registered twice (a wiring bug).
pub fn register_input_commands(
    registry: &mut CommandRegistry,
    sink: TerminalInputSink,
) -> Result<(), RegisterError> {
    for request in TerminalInputCommand::ALL {
        let id = request.id();
        let meta = *command::lookup(id).ok_or(RegisterError::Undeclared(id))?;
        let sink = sink.clone();
        registry.register(meta, move |command: Command, _: HandlerContext| {
            let sink = sink.clone();
            async move {
                let request = TerminalInputCommand::of(&command)
                    .ok_or_else(|| OxiError::validation("not a terminal input command"))?;
                sink.send(request)?;
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
    window.defer(cx, move |window, cx| {
        window.dispatch_action(command.action(), cx)
    });
}
