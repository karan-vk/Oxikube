//! [`ClusterLauncher`]: the app's launcher. Local shells are [`LocalLauncher`]'s; sessions in pod
//! containers (`pod::Shell`, `pod::Attach`, `pod::Exec`, E09-S08) go through the app's
//! [`ExecService`], over the cluster's `ExecPort`.
//!
//! A node shell (`node::Shell`, E09-S09) is opened here too, but only with the permit its guarded
//! command left in the [`ExecService`]: this launcher cannot start one on its own.
//!
//! A pod session is started only by its bus command (the guard has checked the read-only policy
//! and audited the open by then): nothing here restores, clones or retries one by itself, the
//! view dispatches the command again for those.

use std::future::Future;
use std::sync::Arc;

use gpui::{App, Context, Task};
use oxikube_app::{ExecService, ShellOptions};
use oxikube_domain::OxiResult;
use oxikube_domain::ids::ClusterId;
use oxikube_ports::{TerminalBackend, TerminalSize};
use oxikube_runtime::spawn_kube;
use oxikube_workspace::ClusterMark;

use super::TerminalView;
use super::descriptor::BackendDescriptor;
use super::launch::{Launch, LocalLauncher, TerminalLauncher};
use crate::settings::TerminalSettings;

/// Starts local shells and pod sessions. See the module docs.
#[derive(Clone)]
pub struct ClusterLauncher {
    local: LocalLauncher,
    exec: Arc<ExecService>,
}

impl ClusterLauncher {
    /// A launcher for local shells (`local`) and pod sessions (`exec`).
    pub fn new(local: LocalLauncher, exec: Arc<ExecService>) -> Self {
        Self { local, exec }
    }
}

/// Runs `open` on the Kubernetes runtime (aborted when the launch is dropped, so closing the tab
/// while it connects stops the connection) and hands back the backend or why it failed.
fn open_pod_session<Fut>(cx: &mut App, open: Fut) -> Launch
where
    Fut: Future<Output = OxiResult<Box<dyn TerminalBackend>>> + Send + 'static,
{
    let started = spawn_kube(cx, open);
    cx.spawn(async move |_| started.await.unwrap_or_else(|error| Err(error.into())))
}

impl TerminalLauncher for ClusterLauncher {
    fn launch(&self, descriptor: &BackendDescriptor, size: TerminalSize, cx: &mut App) -> Launch {
        let exec = self.exec.clone();
        match descriptor {
            BackendDescriptor::Local { .. } => self.local.launch(descriptor, size, cx),
            BackendDescriptor::Exec {
                pod,
                container,
                command,
            } => {
                let (pod, container, command) = (pod.clone(), container.clone(), command.clone());
                // Read now, on the UI thread: the setting changes apply to the next shell.
                let shells = TerminalSettings::current(cx).exec_shells;
                open_pod_session(cx, async move {
                    if command.is_empty() {
                        let options = ShellOptions::with_shells(shells);
                        exec.open_shell(&pod, container.as_deref(), &options).await
                    } else {
                        exec.exec(&pod, container.as_deref(), &command).await
                    }
                })
            }
            BackendDescriptor::Attach { pod, container } => {
                let (pod, container) = (pod.clone(), container.clone());
                open_pod_session(
                    cx,
                    async move { exec.attach(&pod, container.as_deref()).await },
                )
            }
            BackendDescriptor::NodeShell { node } => {
                // The guarded `node::Shell` left the permit this exchanges for the session; the
                // pod is created, awaited and execed into on the Kubernetes runtime, and deleted
                // when this launch (or the backend it makes) is dropped.
                let node = node.clone();
                open_pod_session(cx, async move { exec.open_node_shell(&node).await })
            }
        }
    }

    fn cluster_mark(&self, cluster: &ClusterId, cx: &App) -> Option<ClusterMark> {
        self.local.cluster_mark(cluster, cx)
    }

    fn follow_mark(&self, cluster: &ClusterId, cx: &mut Context<TerminalView>) -> Option<Task<()>> {
        self.local.follow_mark(cluster, cx)
    }
}
