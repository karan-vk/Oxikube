//! `app::Quit` on the command bus.
//!
//! The `cmd-q` key and the menu run the [`Quit`](super::Quit) action; the palette, the `:q` of the
//! jump bar and an agent send the `app::Quit` command, and it ends in the same
//! [`request_quit`]: with operations running, the confirm dialog asks first.
//! The handler runs off the UI thread, so it only queues the request; the window applies it
//! ([`serve`]).

use futures::StreamExt as _;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use gpui::{App, Task, Window};
use oxikube_app::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};
use oxikube_domain::OxiError;
use oxikube_domain::command::{self, Command, CommandId};

use super::quit::request_quit;

/// A handle on a window's quit queue. Cheap to clone; usable from any thread.
#[derive(Clone, Debug)]
pub struct QuitSink {
    tx: UnboundedSender<()>,
}

impl QuitSink {
    /// A sink and the receiver [`serve`] drains on the UI thread.
    pub fn channel() -> (Self, UnboundedReceiver<()>) {
        let (tx, rx) = unbounded();
        (Self { tx }, rx)
    }
}

/// Registers `app::Quit` on `registry` with its MCP tool stub:
/// `registry.install("oxikube_workspace::quit", |r| register_command(r, sink))`. Quitting changes
/// no cluster, so the command has no guard tier; the confirmation while operations run is the
/// quit guard's.
///
/// # Errors
///
/// A [`RegisterError`] when the id is registered twice (a wiring bug).
pub fn register_command(
    registry: &mut CommandRegistry,
    sink: QuitSink,
) -> Result<(), RegisterError> {
    let meta = *command::lookup(CommandId::APP_QUIT)
        .ok_or(RegisterError::Undeclared(CommandId::APP_QUIT))?;
    registry.register(meta, move |command: Command, _: HandlerContext| {
        let sink = sink.clone();
        async move {
            if command != Command::AppQuit {
                return Err(OxiError::validation("not an app::Quit command"));
            }
            sink.tx
                .unbounded_send(())
                .map_err(|_| OxiError::internal("the window is gone"))?;
            Ok(CommandOutput::none())
        }
    })
}

/// Runs a quit request (asking first while operations run) for each `app::Quit` that arrives on
/// `requests`, until the window closes. Hold the returned task as long as the window lives.
pub fn serve(mut requests: UnboundedReceiver<()>, window: &mut Window, cx: &mut App) -> Task<()> {
    window.spawn(cx, async move |cx| {
        while requests.next().await.is_some() {
            if cx.update(|_, cx| request_quit(cx)).is_err() {
                break;
            }
        }
    })
}
