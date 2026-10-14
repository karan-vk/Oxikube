//! `notifications::TogglePanel` on the command bus (with its MCP tool stub): the handler queues
//! the request and the window toggles its panel on the UI thread ([`serve`]). The key binding
//! runs the same toggle through the panel's toggle action, which the workspace handles.

use futures::StreamExt as _;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use gpui::{App, Task, WeakEntity, Window};
use oxikube_app::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};
use oxikube_domain::OxiError;
use oxikube_domain::command::{self, Command, CommandId};

use super::panel::NotificationPanel;
use crate::workspace::Workspace;

/// Where `notifications::TogglePanel` sends its requests.
#[derive(Clone, Debug)]
pub struct NotificationPanelSink {
    tx: UnboundedSender<()>,
}

impl NotificationPanelSink {
    /// A sink and the receiver the window drains ([`serve`]).
    pub fn channel() -> (Self, UnboundedReceiver<()>) {
        let (tx, rx) = unbounded();
        (Self { tx }, rx)
    }

    /// Asks the window to toggle its panel. Returns false when the window is gone.
    pub fn request(&self) -> bool {
        self.tx.unbounded_send(()).is_ok()
    }
}

/// Registers `notifications::TogglePanel` on `registry`:
/// `registry.install("oxikube_workspace::notifications", |r| register_commands(r, sink))`.
///
/// # Errors
///
/// A [`RegisterError`] when the id is registered twice (a wiring bug).
pub fn register_commands(
    registry: &mut CommandRegistry,
    sink: NotificationPanelSink,
) -> Result<(), RegisterError> {
    let id = CommandId::NOTIFICATIONS_TOGGLE_PANEL;
    let meta = *command::lookup(id).ok_or(RegisterError::Undeclared(id))?;
    registry.register(meta, move |_: Command, _: HandlerContext| {
        let sink = sink.clone();
        async move {
            if !sink.request() {
                return Err(OxiError::internal("the main window is gone"));
            }
            Ok(CommandOutput::none())
        }
    })
}

/// Toggles `workspace`'s notification panel for each request, on the UI thread. Keep the task
/// as long as the window lives.
pub fn serve(
    mut requests: UnboundedReceiver<()>,
    workspace: WeakEntity<Workspace>,
    window: &mut Window,
    cx: &mut App,
) -> Task<()> {
    window.spawn(cx, async move |cx| {
        while requests.next().await.is_some() {
            let toggled = cx.update(|window, cx| {
                workspace.update(cx, |ws, cx| {
                    ws.toggle_panel::<NotificationPanel>(window, cx)
                })
            });
            if !matches!(toggled, Ok(Ok(_))) {
                return;
            }
        }
    })
}
