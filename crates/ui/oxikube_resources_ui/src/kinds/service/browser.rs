//! Whether "Open in browser" is offered: the hook is a port-forward manager (E15), and until one
//! is installed the buttons are not drawn.
//!
//! The wiring that installs the manager does two things, both here:
//! [`register_open_in_browser`] puts the handler of `service::OpenInBrowser` on the bus (so the
//! row action and the palette entry appear, and an agent can call it), and
//! [`ServiceBrowser::enable`] tells this window's Service panels to draw their per-port buttons.

use std::sync::Arc;

use gpui::{App, Global};
use oxikube_app::command_bus::{CommandRegistry, RegisterError};
use oxikube_app::{ServicePortOpener, UrlSink};

use crate::views::{ResourceCommandSink, ViewRequest};

/// The window's switch for the Service panel's "Open in browser" buttons. A GPUI global.
#[derive(Default)]
pub struct ServiceBrowser {
    enabled: bool,
}

impl Global for ServiceBrowser {}

impl ServiceBrowser {
    /// Shows "Open in browser" on each TCP port of a Service panel. Call it where the opener is
    /// installed (see the module docs).
    pub fn enable(cx: &mut App) {
        cx.default_global::<ServiceBrowser>().enabled = true;
    }

    /// Hides the buttons again.
    pub fn disable(cx: &mut App) {
        cx.default_global::<ServiceBrowser>().enabled = false;
    }

    /// Whether the buttons are drawn.
    pub fn is_enabled(cx: &App) -> bool {
        cx.try_global::<ServiceBrowser>().is_some_and(|b| b.enabled)
    }
}

/// Registers the handler of `service::OpenInBrowser` over `opener`: it forwards the port, then
/// asks the window (through `sink`, on the UI thread) to open the URL.
///
/// # Errors
///
/// [`RegisterError`] when the command is already registered.
pub fn register_open_in_browser(
    registry: &mut CommandRegistry,
    opener: Arc<dyn ServicePortOpener>,
    sink: ResourceCommandSink,
) -> Result<(), RegisterError> {
    let urls: UrlSink = Arc::new(move |url| {
        sink.send(ViewRequest::OpenUrl(url));
    });
    oxikube_app::register_open_in_browser(registry, opener, urls)
}
