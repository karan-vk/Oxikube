//! `notifications::TogglePanel` and `notifications::OpenLink` on the command bus (each with its
//! MCP tool stub): the handler queues the request and the window serves it on the UI thread
//! ([`serve`]): toggling its panel, or opening an `https` link in the browser. The key binding
//! runs the same toggle through the panel's toggle action, which the workspace handles.

use futures::StreamExt as _;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use gpui::{App, Task, WeakEntity, Window};
use oxikube_app::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};
use oxikube_domain::OxiError;
use oxikube_domain::command::{self, Command, CommandId};

use super::panel::NotificationPanel;
use crate::workspace::Workspace;

/// What the window is asked to do for the notification commands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PanelRequest {
    /// Show and focus the panel, or hide it.
    Toggle,
    /// Open this `https` link in the browser.
    OpenLink(String),
}

/// Where the notification commands send their requests.
#[derive(Clone, Debug)]
pub struct NotificationPanelSink {
    tx: UnboundedSender<PanelRequest>,
}

impl NotificationPanelSink {
    /// A sink and the receiver the window drains ([`serve`]).
    pub fn channel() -> (Self, UnboundedReceiver<PanelRequest>) {
        let (tx, rx) = unbounded();
        (Self { tx }, rx)
    }

    /// Asks the window to toggle its panel. Returns false when the window is gone.
    pub fn request(&self) -> bool {
        self.tx.unbounded_send(PanelRequest::Toggle).is_ok()
    }

    /// Asks the window to open `url` in the browser. Returns false when the window is gone.
    pub fn open_link(&self, url: String) -> bool {
        self.tx.unbounded_send(PanelRequest::OpenLink(url)).is_ok()
    }
}

/// Registers `notifications::TogglePanel` and `notifications::OpenLink` on `registry`:
/// `registry.install("oxikube_workspace::notifications", |r| register_commands(r, sink))`.
///
/// # Errors
///
/// A [`RegisterError`] when an id is registered twice (a wiring bug).
pub fn register_commands(
    registry: &mut CommandRegistry,
    sink: NotificationPanelSink,
) -> Result<(), RegisterError> {
    for id in [
        CommandId::NOTIFICATIONS_TOGGLE_PANEL,
        CommandId::NOTIFICATIONS_OPEN_LINK,
    ] {
        let meta = *command::lookup(id).ok_or(RegisterError::Undeclared(id))?;
        let sink = sink.clone();
        registry.register(meta, move |command: Command, _: HandlerContext| {
            let sink = sink.clone();
            async move {
                let sent = match command {
                    Command::NotificationsOpenLink { url } => {
                        if !is_https_link(&url) {
                            return Err(OxiError::validation("only https links can be opened"));
                        }
                        sink.open_link(url)
                    }
                    _ => sink.request(),
                };
                if !sent {
                    return Err(OxiError::internal("the main window is gone"));
                }
                Ok(CommandOutput::none())
            }
        })?;
    }
    Ok(())
}

/// Whether `url` is an `https` link without whitespace or control characters.
fn is_https_link(url: &str) -> bool {
    url.strip_prefix("https://").is_some_and(|rest| {
        !rest.is_empty() && !url.chars().any(|c| c.is_whitespace() || c.is_control())
    })
}

/// Serves each request on the UI thread: toggles `workspace`'s notification panel, or opens a
/// link. Keep the task as long as the window lives.
pub fn serve(
    mut requests: UnboundedReceiver<PanelRequest>,
    workspace: WeakEntity<Workspace>,
    window: &mut Window,
    cx: &mut App,
) -> Task<()> {
    window.spawn(cx, async move |cx| {
        while let Some(request) = requests.next().await {
            let PanelRequest::Toggle = request else {
                if let PanelRequest::OpenLink(url) = request {
                    // The window may be gone; then there is nobody to open it for.
                    let _ = cx.update(|_, cx| cx.open_url(&url));
                }
                continue;
            };
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
