//! `portforward::CopyUrl`, `portforward::OpenInBrowser` and `portforward::TogglePanel` on the
//! command bus (each with its MCP tool stub). The three need the window (the clipboard, the system
//! browser, the dock), so a handler checks its forward, queues a [`UiRequest`] and the window
//! serves it on the UI thread ([`serve`]). `Stop`, `Restart` and `Remove` are the app's
//! (`oxikube_app::portforward`): they only act on the manager.
//!
//! None changes a cluster, so none goes through a confirmation tier. Opening only ever hands an
//! `http` / `https` URL of a loopback (or the configured bind) address to the system browser:
//! never a webview.

use std::sync::Arc;

use futures::StreamExt as _;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use gpui::{App, ClipboardItem, Task, WeakEntity, Window};
use oxikube_app::ForwardId;
use oxikube_app::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};
use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::{OxiError, OxiResult};
use oxikube_workspace::{Toast, Workspace};
use serde_json::json;

use crate::panel::PortForwardPanel;
use crate::source::ForwardSource;

/// What the window is asked to do for the panel's commands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UiRequest {
    /// Show and focus the panel (`show`), or toggle it.
    Toggle {
        /// Only ever open it.
        show: bool,
    },
    /// Put this URL on the clipboard.
    CopyUrl(String),
    /// Open this URL in the system browser.
    OpenUrl(String),
}

/// Where the panel's commands send their requests.
#[derive(Clone, Debug)]
pub struct PortForwardUiSink {
    tx: UnboundedSender<UiRequest>,
}

impl PortForwardUiSink {
    /// A sink and the receiver the window drains ([`serve`]).
    pub fn channel() -> (Self, UnboundedReceiver<UiRequest>) {
        let (tx, rx) = unbounded();
        (Self { tx }, rx)
    }

    /// Queues `request`. Returns false when the window is gone.
    pub fn send(&self, request: UiRequest) -> bool {
        self.tx.unbounded_send(request).is_ok()
    }
}

/// The commands [`register_commands`] installs.
pub const COMMANDS: [CommandId; 3] = [
    CommandId::PORTFORWARD_COPY_URL,
    CommandId::PORTFORWARD_OPEN_IN_BROWSER,
    CommandId::PORTFORWARD_TOGGLE_PANEL,
];

/// Registers the panel's commands on `registry`:
/// `registry.install("oxikube_portforward_ui", |r| register_commands(r, source, sink))`.
///
/// # Errors
///
/// A [`RegisterError`] when an id is registered twice (a wiring bug).
pub fn register_commands(
    registry: &mut CommandRegistry,
    source: Arc<dyn ForwardSource>,
    sink: PortForwardUiSink,
) -> Result<(), RegisterError> {
    for id in COMMANDS {
        let meta = *command::lookup(id).ok_or(RegisterError::Undeclared(id))?;
        let (source, sink) = (source.clone(), sink.clone());
        registry.register(meta, move |command: Command, _: HandlerContext| {
            let (source, sink) = (source.clone(), sink.clone());
            async move { run(&*source, &sink, command) }
        })?;
    }
    Ok(())
}

pub(crate) fn run(
    source: &dyn ForwardSource,
    sink: &PortForwardUiSink,
    command: Command,
) -> OxiResult<CommandOutput> {
    let (request, message) = match command {
        Command::PortForwardTogglePanel { show } => (UiRequest::Toggle { show }, None),
        Command::PortForwardCopyUrl { id } => {
            let url = url_of(source, id)?;
            let message = format!("Copied {url}");
            (UiRequest::CopyUrl(url), Some(message))
        }
        Command::PortForwardOpenInBrowser { id } => {
            let url = url_of(source, id)?;
            let message = format!("Opening {url}");
            (UiRequest::OpenUrl(url), Some(message))
        }
        other => {
            return Err(OxiError::internal(format!(
                "{} is not a port-forward panel command",
                other.id()
            )));
        }
    };
    let data = match &request {
        UiRequest::CopyUrl(url) | UiRequest::OpenUrl(url) => Some(json!({ "url": url })),
        UiRequest::Toggle { .. } => None,
    };
    if !sink.send(request) {
        return Err(OxiError::internal("the main window is gone"));
    }
    Ok(CommandOutput { message, data })
}

/// The local URL of forward `id`, or why it has none.
fn url_of(source: &dyn ForwardSource, id: u64) -> OxiResult<String> {
    let info = source
        .get(ForwardId(id))
        .ok_or_else(|| OxiError::not_found(format!("no port-forward {id}")))?;
    info.url().ok_or_else(|| {
        OxiError::conflict(format!(
            "Port-forward {} is not listening yet: it has no local address",
            info.label()
        ))
    })
}

/// Serves each request on the UI thread: toggles `workspace`'s Port Forwards panel, copies a URL
/// (with a toast) or opens one in the system browser. Keep the task as long as the window lives.
pub fn serve(
    mut requests: UnboundedReceiver<UiRequest>,
    workspace: WeakEntity<Workspace>,
    window: &mut Window,
    cx: &mut App,
) -> Task<()> {
    window.spawn(cx, async move |cx| {
        while let Some(request) = requests.next().await {
            let workspace = workspace.clone();
            let served = cx.update(|window, cx| match request {
                UiRequest::Toggle { show } => {
                    workspace
                        .update(cx, |ws, cx| {
                            let shown = ws.toggle_panel::<PortForwardPanel>(window, cx);
                            // Shown and focused already: the toggle hid it, `show` wants it up.
                            if show && !shown {
                                ws.toggle_panel::<PortForwardPanel>(window, cx);
                            }
                        })
                        .ok();
                }
                UiRequest::CopyUrl(url) => {
                    cx.write_to_clipboard(ClipboardItem::new_string(url.clone()));
                    workspace
                        .update(cx, |ws, cx| {
                            ws.show_toast(Toast::info(format!("Copied {url}")), cx);
                        })
                        .ok();
                }
                UiRequest::OpenUrl(url) => cx.open_url(&url),
            });
            if served.is_err() {
                return;
            }
        }
    })
}
