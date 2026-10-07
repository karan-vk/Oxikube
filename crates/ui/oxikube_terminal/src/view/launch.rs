//! Starting the process a [`BackendDescriptor`] describes: [`TerminalLauncher`], and the app's
//! [`LocalLauncher`] for local shells.
//!
//! A [`TerminalView`](super::TerminalView) never builds a backend itself: it asks its launcher,
//! so the tests start a `FakeTerminalBackend`, the app a `LocalPty`, and pod terminals
//! (`ExecService`, E09-S08) plug in the same way. Starting is always off the UI thread.

use std::sync::Arc;

use gpui::{App, Context, Task};
use oxikube_app::ClusterSessionManager;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::cluster_source::ClusterSourcePort;
use oxikube_ports::{TerminalBackend, TerminalSize};
use oxikube_runtime::spawn_kube;
use oxikube_workspace::ClusterMark;
use oxikube_workspace::cluster::follow_session;

use super::TerminalView;
use super::descriptor::BackendDescriptor;
use crate::backend::local::{ClusterEnv, LocalPty, LocalPtyOptions, files_of_sources};
use crate::settings::TerminalSettings;

/// The result of a launch: the running backend, or why it could not start.
pub type Launch = Task<OxiResult<Box<dyn TerminalBackend>>>;

/// Starts terminal processes. See the [module docs](self).
pub trait TerminalLauncher: 'static {
    /// Starts the process `descriptor` describes, at `size`. The work runs off the UI thread;
    /// dropping the task abandons it (a backend that started anyway is dropped, which ends it).
    fn launch(&self, descriptor: &BackendDescriptor, size: TerminalSize, cx: &mut App) -> Launch;

    /// The colour and read-only mark of `cluster`'s tab, shown on its terminals' tabs. `None`
    /// draws nothing.
    fn cluster_mark(&self, cluster: &ClusterId, cx: &App) -> Option<ClusterMark> {
        let _ = (cluster, cx);
        None
    }

    /// Keeps the mark on a terminal's tab current: a task that calls
    /// [`TerminalView::set_cluster_mark`] whenever `cluster`'s read-only flag or colour changes
    /// (or its session closes). The view keeps the task; dropping it stops following. `None`
    /// (the default) when marks never change.
    fn follow_mark(&self, cluster: &ClusterId, cx: &mut Context<TerminalView>) -> Option<Task<()>> {
        let _ = (cluster, cx);
        None
    }
}

/// The app's launcher: local shells on a PTY ([`LocalPty`]), a cluster shell with the cluster's
/// kubeconfig, context and namespace in its environment.
///
/// The context comes from the cluster's session, the kubeconfig files from the catalog's sources
/// (read again at every start, so a restored terminal never relies on anything saved). Pod
/// terminals are not started here.
#[derive(Clone)]
pub struct LocalLauncher {
    sessions: ClusterSessionManager,
    sources: Arc<dyn ClusterSourcePort>,
}

impl LocalLauncher {
    /// A launcher over the app's sessions (cluster -> context, tab mark) and kubeconfig sources.
    pub fn new(sessions: ClusterSessionManager, sources: Arc<dyn ClusterSourcePort>) -> Self {
        Self { sessions, sources }
    }

    fn launch_local(
        &self,
        descriptor: &BackendDescriptor,
        size: TerminalSize,
        cx: &mut App,
    ) -> OxiResult<Launch> {
        let BackendDescriptor::Local {
            cluster,
            namespace,
            shell,
            args,
            cwd,
        } = descriptor
        else {
            return Err(OxiError::unsupported(
                "pod terminals cannot be opened in this build",
            ));
        };
        let mut options = LocalPtyOptions::from_settings(&TerminalSettings::current(cx));
        if let Some(shell) = shell {
            options.shell = Some(shell.clone());
            options.args = args.clone();
        }
        options.cwd = cwd.clone();
        options.size = size;
        let context = match cluster {
            Some(cluster) => {
                let session = self
                    .sessions
                    .get(cluster)
                    .ok_or_else(|| OxiError::not_found("the terminal's cluster is not open"))?;
                Some(session.context().clone())
            }
            None => None,
        };
        let sources = self.sources.clone();
        let namespace = namespace.clone();
        let started = spawn_kube(cx, async move {
            let files = match context {
                Some(_) => Some(sources.sources().await?),
                None => None,
            };
            // Reading the kubeconfig sources' folders and forking block: off the async threads.
            let spawned = tokio::task::spawn_blocking(move || {
                if let (Some(context), Some(sources)) = (context, files) {
                    let env = ClusterEnv {
                        context,
                        namespace,
                        kubeconfig_files: files_of_sources(&sources),
                    };
                    options.cluster = Some(env);
                }
                LocalPty::spawn(options)
            })
            .await
            .map_err(|_| OxiError::internal("the terminal could not be started"))??;
            Ok(Box::new(spawned) as Box<dyn TerminalBackend>)
        });
        Ok(cx.spawn(async move |_| match started.await {
            Ok(result) => result,
            Err(error) => Err(OxiError::internal(error.to_string())),
        }))
    }
}

impl TerminalLauncher for LocalLauncher {
    fn launch(&self, descriptor: &BackendDescriptor, size: TerminalSize, cx: &mut App) -> Launch {
        self.launch_local(descriptor, size, cx)
            .unwrap_or_else(|error| Task::ready(Err(error)))
    }

    fn cluster_mark(&self, cluster: &ClusterId, _: &App) -> Option<ClusterMark> {
        self.sessions
            .get(cluster)
            .map(|session| ClusterMark::of(&session))
    }

    fn follow_mark(&self, cluster: &ClusterId, cx: &mut Context<TerminalView>) -> Option<Task<()>> {
        let cluster = cluster.clone();
        Some(follow_session(
            &self.sessions,
            cx,
            move |_| Some(cluster.clone()),
            |view, session, cx| {
                view.set_cluster_mark(session.as_ref().map(ClusterMark::of), cx);
            },
        ))
    }
}
