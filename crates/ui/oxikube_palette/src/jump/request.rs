//! The doors into the jump bar from the command bus: [`JumpRequest`], the [`JumpSink`] a window
//! drains, and [`register_commands`].

use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use oxikube_app::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};
use oxikube_app::search::jump::HistoryStep;
use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::{OxiError, OxiResult};

/// What a door asks the jump bar to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JumpRequest {
    /// Open the bar (`palette::OpenJump`), or close it when it is open.
    Open,
    /// Run an earlier line again (`jump::Back`, `jump::Forward`, `jump::Last`).
    Step(HistoryStep),
}

impl JumpRequest {
    /// The request a bus [`Command`] makes; `None` for any other command.
    pub fn of(command: &Command) -> Option<Self> {
        match command {
            Command::PaletteOpenJump => Some(Self::Open),
            Command::JumpBack => Some(Self::Step(HistoryStep::Back)),
            Command::JumpForward => Some(Self::Step(HistoryStep::Forward)),
            Command::JumpLast => Some(Self::Step(HistoryStep::Last)),
            _ => None,
        }
    }

    /// The commands the host serves.
    pub const COMMANDS: [CommandId; 4] = [
        CommandId::PALETTE_OPEN_JUMP,
        CommandId::JUMP_BACK,
        CommandId::JUMP_FORWARD,
        CommandId::JUMP_LAST,
    ];
}

/// A handle on a window's jump queue. Cheap to clone; usable from any thread.
#[derive(Clone, Debug)]
pub struct JumpSink {
    tx: UnboundedSender<JumpRequest>,
}

impl JumpSink {
    /// A sink and the receiver the window drains on the UI thread ([`super::JumpHost::serve`]).
    pub fn channel() -> (Self, UnboundedReceiver<JumpRequest>) {
        let (tx, rx) = unbounded();
        (Self { tx }, rx)
    }

    fn send(&self, request: JumpRequest) -> OxiResult<()> {
        self.tx
            .unbounded_send(request)
            .map_err(|_| OxiError::internal("the window with the jump bar is gone"))
    }
}

/// Registers [`JumpRequest::COMMANDS`] on `registry`, each with its MCP tool stub:
/// `registry.install("oxikube_palette::jump", |r| register_commands(r, sink))`. The handlers only
/// queue the request; the window applies it on the UI thread. None changes a cluster, so none
/// has a guard tier.
///
/// # Errors
///
/// A [`RegisterError`] when an id is registered twice (a wiring bug).
pub fn register_commands(
    registry: &mut CommandRegistry,
    sink: JumpSink,
) -> Result<(), RegisterError> {
    for id in JumpRequest::COMMANDS {
        let meta = *command::lookup(id).ok_or(RegisterError::Undeclared(id))?;
        let sink = sink.clone();
        registry.register(meta, move |command: Command, _: HandlerContext| {
            let sink = sink.clone();
            async move {
                let request = JumpRequest::of(&command)
                    .ok_or_else(|| OxiError::validation("not a jump bar command"))?;
                sink.send(request)?;
                Ok(CommandOutput::none())
            }
        })?;
    }
    Ok(())
}
