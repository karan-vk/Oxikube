//! [`SourcesBackend`]: where the sources screen reads its rows and sends its commands.

use std::path::PathBuf;

use futures::stream::BoxStream;
use gpui::{App, Task};
use oxikube_app::KubeconfigSourcesService;
use oxikube_app::command_bus::CommandOutput;
use oxikube_app::sources::SourceRow;
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::Command;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{SourcesChanged, UserSource};
use oxikube_runtime::spawn_kube;

/// What the sources screen needs from the app. The view never touches a file, a settings file
/// or a port: it reads rows and sends [`Command`]s, and shows what comes back (non-negotiable 4).
///
/// The production implementation, [`ServiceBackend`], runs the `kubeconfig::*` commands on the
/// [`KubeconfigSourcesService`] on the Tokio bridge until the binary mounts the `CommandBus`
/// (E06-S02), the way the catalog's `ServiceDispatcher` does; the same handlers are registered
/// for the bus with `oxikube_app::sources::register_commands`. Tests run the real service over
/// fakes.
pub trait SourcesBackend: 'static {
    /// Reads the list with each source's status. Local work, off the UI thread.
    fn rows(&self, cx: &mut App) -> Task<OxiResult<Vec<SourceRow>>>;

    /// Runs a `kubeconfig::*` command and returns its output (a message and the new rows).
    /// The task keeps running to its end even when the caller stops waiting for it, so a file
    /// being written is never left half done.
    fn run(&self, command: Command, cx: &mut App) -> Task<OxiResult<CommandOutput>>;

    /// Whether removing `source` deletes a file (one Oxikube stored) rather than only the
    /// entry. The confirmation text depends on it. Instant: no file access.
    fn deletes_file(&self, source: &UserSource) -> bool;

    /// Where pasted kubeconfigs are stored, for the paste dialog's note.
    fn stored_dir(&self) -> PathBuf;

    /// The catalog's changes from now on: a new read of the sources shows up here.
    fn changes(&self) -> BoxStream<'static, SourcesChanged>;
}

/// The [`SourcesBackend`] over the real [`KubeconfigSourcesService`].
#[derive(Clone, Debug)]
pub struct ServiceBackend {
    service: KubeconfigSourcesService,
}

impl ServiceBackend {
    /// A backend over `service`.
    pub fn new(service: KubeconfigSourcesService) -> Self {
        Self { service }
    }
}

impl SourcesBackend for ServiceBackend {
    fn rows(&self, cx: &mut App) -> Task<OxiResult<Vec<SourceRow>>> {
        let service = self.service.clone();
        cx.spawn(async move |cx| {
            spawn_kube(&*cx, async move { service.rows().await })
                .await
                .map_err(OxiError::from)
                .and_then(|inner| inner)
        })
    }

    fn run(&self, command: Command, cx: &mut App) -> Task<OxiResult<CommandOutput>> {
        let service = self.service.clone();
        cx.spawn(async move |cx| {
            spawn_kube(&*cx, async move {
                service.execute(&command, Initiator::Ui).await
            })
            .await
            .map_err(OxiError::from)
            .and_then(|inner| inner)
        })
    }

    fn deletes_file(&self, source: &UserSource) -> bool {
        self.service.deletes_file_on_remove(source)
    }

    fn stored_dir(&self) -> PathBuf {
        self.service.kubeconfigs_dir().to_path_buf()
    }

    fn changes(&self) -> BoxStream<'static, SourcesChanged> {
        self.service.changes()
    }
}
