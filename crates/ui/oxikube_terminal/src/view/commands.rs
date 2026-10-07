//! `terminal::New`, `terminal::Split`, `terminal::Close`, `terminal::Reconnect` and
//! `terminal::Restart` on the command bus, and the pod commands `pod::Shell`, `pod::Attach` and
//! `pod::Exec` (E09-S08) that open a terminal in a container.
//!
//! The keymap (through the GPUI actions of the same names), the terminal panel's button, the
//! palette and agents (`app.terminal_new`, ...) all dispatch the bus command. Its handler queues
//! a [`TerminalRequest`] on the window's [`TerminalViewSink`]; the window's
//! [`TerminalViews`](super::TerminalViews) applies it on the UI thread. Nothing here reads or
//! changes a cluster: no `MutationGuard` tier (a cluster shell's commands are the user's own).
//!
//! The pod commands are exec-class: the bus's guard applies the read-only block and the audit
//! record before the handler here runs (the handler only queues the request), and the terminal
//! then connects through the launcher, so a failed connection shows in the tab.

use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use oxikube_app::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};
use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::ids::{ClusterId, ResourceRef};
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
    /// `pod::Shell`, `pod::Attach`, `pod::Exec`, or the terminal of a new debug container
    /// (`pod::Debug`, E09-S10, as an attach of it): a terminal in a pod's container, in the bottom
    /// dock of its cluster's tab. The descriptor is [`BackendDescriptor::Exec`] (an empty
    /// command means the shell chain) or [`BackendDescriptor::Attach`].
    Pod(BackendDescriptor),
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

    pub(super) fn send(&self, request: TerminalRequest) -> OxiResult<()> {
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

/// The commands [`register_pod_commands`] handles.
pub const POD_COMMANDS: [CommandId; 3] = [
    CommandId::POD_SHELL,
    CommandId::POD_ATTACH,
    CommandId::POD_EXEC,
];

/// The terminal a pod command asks for.
///
/// # Errors
///
/// `Validation` when the target is not a pod, or for an empty program in `pod::Exec`'s argv.
fn pod_request(command: Command) -> OxiResult<TerminalRequest> {
    let descriptor = match command {
        Command::PodShell { target, container } => {
            ensure_pod(&target, "pod::Shell")?;
            BackendDescriptor::Exec {
                pod: target,
                container: blank_to_none(container),
                command: Vec::new(),
            }
        }
        Command::PodAttach { target, container } => {
            ensure_pod(&target, "pod::Attach")?;
            BackendDescriptor::Attach {
                pod: target,
                container: blank_to_none(container),
            }
        }
        Command::PodExec {
            target,
            container,
            command,
        } => {
            ensure_pod(&target, "pod::Exec")?;
            if command
                .first()
                .is_some_and(|program| program.trim().is_empty())
            {
                return Err(OxiError::validation("pod::Exec needs a program to run"));
            }
            BackendDescriptor::Exec {
                pod: target,
                container: blank_to_none(container),
                command,
            }
        }
        _ => return Err(OxiError::validation("not a pod terminal command")),
    };
    Ok(TerminalRequest::Pod(descriptor))
}

fn blank_to_none(container: Option<String>) -> Option<String> {
    container.filter(|name| !name.trim().is_empty())
}

pub(super) fn ensure_pod(target: &ResourceRef, command: &str) -> OxiResult<()> {
    if !target.gvk.is_pod() || target.namespace.is_none() {
        return Err(OxiError::validation(format!(
            "{command} needs a namespaced pod, not a {}",
            target.gvk.kind
        )));
    }
    Ok(())
}

/// Registers `pod::Shell`, `pod::Attach` and `pod::Exec` on `registry` (each with its MCP tool
/// stub: unsafe, interactive, hidden from agents by default). Each handler queues a
/// [`TerminalRequest::Pod`] on `sink`; the window's [`TerminalViews`](super::TerminalViews) opens
/// the terminal. Call it with the same sink as [`register_view_commands`].
///
/// # Errors
///
/// A [`RegisterError`] when an id is registered twice (a wiring bug).
pub fn register_pod_commands(
    registry: &mut CommandRegistry,
    sink: TerminalViewSink,
) -> Result<(), RegisterError> {
    for id in POD_COMMANDS {
        let meta = *command::lookup(id).ok_or(RegisterError::Undeclared(id))?;
        let sink = sink.clone();
        registry.register(meta, move |command: Command, _: HandlerContext| {
            let sink = sink.clone();
            async move {
                sink.send(pod_request(command)?)?;
                Ok(CommandOutput::message("opening the terminal"))
            }
        })?;
    }
    Ok(())
}
