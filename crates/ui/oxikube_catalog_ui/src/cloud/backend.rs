//! [`CloudBackend`]: where the Cloud section sends its commands and reads discovery from.

use std::sync::Arc;

use futures::stream::BoxStream;
use gpui::{App, Task};
use oxikube_app::CloudDiscoveryService;
use oxikube_app::cloud::{CloudDiscoveryEvent, ManagedEntry};
use oxikube_app::command_bus::CommandOutput;
use oxikube_domain::cloud::CloudProvider;
use oxikube_domain::command::Command;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_runtime::spawn_kube;

/// What the Cloud section needs from the app. The view never runs a CLI or touches a file: it
/// sends `cloud::*` [`Command`]s and shows the events and the added clusters that come back
/// (non-negotiable 4).
///
/// The production implementation, [`ServiceBackend`], runs the commands on the
/// [`CloudDiscoveryService`] on the Tokio bridge, the way the sources screen does; the same
/// handlers are registered for the bus with `oxikube_app::cloud::register_commands`, so a key or
/// the palette starts the same runs and the section follows them through
/// [`watch`](Self::watch). Tests run a scripted backend or the real service over fakes.
pub trait CloudBackend: 'static {
    /// Runs a `cloud::*` command and returns its output. The task runs to its end even when the
    /// caller stops waiting for it, so a managed file is never left half written.
    fn run(&self, command: Command, cx: &mut App) -> Task<OxiResult<CommandOutput>>;

    /// Every step of the refreshes from now on, whoever started them.
    fn watch(&self) -> BoxStream<'static, Arc<CloudDiscoveryEvent>>;

    /// The clusters in the managed kubeconfig of `provider` ("in the catalog"). Local file work,
    /// off the UI thread.
    fn added(&self, provider: CloudProvider, cx: &mut App) -> Task<OxiResult<Vec<ManagedEntry>>>;
}

/// The [`CloudBackend`] over the real [`CloudDiscoveryService`].
#[derive(Clone, Debug)]
pub struct ServiceBackend {
    service: CloudDiscoveryService,
}

impl ServiceBackend {
    /// A backend over `service`.
    pub fn new(service: CloudDiscoveryService) -> Self {
        Self { service }
    }
}

impl CloudBackend for ServiceBackend {
    fn run(&self, command: Command, cx: &mut App) -> Task<OxiResult<CommandOutput>> {
        let service = self.service.clone();
        cx.spawn(async move |cx| {
            spawn_kube(&*cx, async move { service.execute(&command).await })
                .await
                .map_err(OxiError::from)
                .and_then(|inner| inner)
        })
    }

    fn watch(&self) -> BoxStream<'static, Arc<CloudDiscoveryEvent>> {
        self.service.watch()
    }

    fn added(&self, provider: CloudProvider, cx: &mut App) -> Task<OxiResult<Vec<ManagedEntry>>> {
        let service = self.service.clone();
        cx.spawn(async move |cx| {
            spawn_kube(&*cx, async move { service.added(provider).await })
                .await
                .map_err(OxiError::from)
                .and_then(|inner| inner)
        })
    }
}
