//! The hook behind "Open in browser" (`service::OpenInBrowser`).
//!
//! The port-forward manager (E15) implements [`ServicePortOpener`]: it picks a pod that backs the
//! Service, forwards a local port to the pod port the Service port maps to and says where to
//! point the browser. Until it exists nothing registers the handler, and the command is not
//! offered anywhere: [`RowActions`](crate::RowActions) keeps only the actions whose command is on
//! the bus, and the Service detail hides its buttons.

use std::sync::Arc;

use async_trait::async_trait;
use oxikube_domain::OxiError;
use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::ids::ResourceRef;

use crate::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};

/// A forward that is up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenedServicePort {
    /// The URL to open: `http://localhost:<local port>` (or `https://`; build it with
    /// [`ServicePortSummary::browser_url`](oxikube_domain::view::ServicePortSummary::browser_url)).
    pub url: String,
    /// The Service port that was forwarded (the first TCP port when the command named none).
    pub service_port: u16,
}

/// Starts a port-forward to a Service's backing pod for a browser to use. Implemented by E15's
/// port-forward manager.
#[async_trait]
pub trait ServicePortOpener: Send + Sync {
    /// Forwards `port` of `service` (its first TCP port when `None`) to a local port. The
    /// forward is the manager's to guard, audit and list; this only asks for it.
    ///
    /// # Errors
    ///
    /// No backing pod is ready, the Service has no TCP port, or the forward failed.
    async fn open_service_port(
        &self,
        service: ResourceRef,
        port: Option<u16>,
    ) -> Result<OpenedServicePort, OxiError>;
}

/// Where the opened URL goes: the UI thread, which hands it to the operating system's browser.
pub type UrlSink = Arc<dyn Fn(String) + Send + Sync>;

/// Registers the handler of `service::OpenInBrowser` over `opener`. The handler forwards the port,
/// sends the URL to `urls` and reports it as the command's output.
///
/// # Errors
///
/// [`RegisterError`] when the command is undeclared or already registered.
pub fn register_open_in_browser(
    registry: &mut CommandRegistry,
    opener: Arc<dyn ServicePortOpener>,
    urls: UrlSink,
) -> Result<(), RegisterError> {
    let meta = command::lookup(CommandId::SERVICE_OPEN_IN_BROWSER)
        .copied()
        .ok_or(RegisterError::Undeclared(
            CommandId::SERVICE_OPEN_IN_BROWSER,
        ))?;
    registry.register(meta, move |command: Command, _: HandlerContext| {
        let (opener, urls) = (opener.clone(), urls.clone());
        async move {
            let Command::ServiceOpenInBrowser { target, port } = command else {
                return Err(OxiError::validation("not a service::OpenInBrowser command"));
            };
            let opened = opener.open_service_port(target, port).await?;
            urls(opened.url.clone());
            Ok(CommandOutput {
                message: Some(format!("Opened {}", opened.url)),
                data: Some(serde_json::json!({
                    "url": opened.url,
                    "service_port": opened.service_port,
                })),
            })
        }
    })
}
