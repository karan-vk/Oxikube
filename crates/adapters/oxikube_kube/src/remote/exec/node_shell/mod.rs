//! Node shells: a shell on a node through a privileged pod (E04-S09).
//!
//! The pod ([`node_shell_manifest`], rendered from the
//! [`NodeShellSpec`] in `oxikube_ports`) is created on the target node, the shell is an exec of
//! `nsenter` into the node's namespaces ([`node_shell_command`]), and the pod is deleted when the
//! shell exits, when opening fails at any step, and when the session is dropped or aborted
//! (`guard`), or when the app quits (`LiveShells`, `release`). While a shell is open its pod
//! is stamped alive every minute (`heartbeat`). If a run dies before it can clean up (a crash, a
//! lost connection), `sweep` deletes what it left once nothing has stamped the pod for a while,
//! which never takes another window's or user's live shell, and the pod's
//! `activeDeadlineSeconds` ends it regardless.
//!
//! The template, its defaults and the manifest live in the ports crate so the app can show and
//! dry-run exactly the pod this creates (E09-S09); this module owns the cluster side: create,
//! wait, exec, delete.
//!
//! | Piece | Where |
//! |---|---|
//! | cleanup on end, error and drop | `guard` |
//! | stamping the pod alive while the shell is open | `heartbeat` |
//! | the shells open now, for the app's quit | `live` |
//! | `open`, `sweep` | this file |

mod guard;
mod heartbeat;
mod live;

use std::sync::Arc;
use std::time::Duration;

use oxikube_domain::OxiResult;
use oxikube_ports::{
    ExecOptions, ExecSession, ExecStreamPort, NODE_SHELL_CONTAINER, NODE_SHELL_LABEL,
    NodeShellSpec, node_shell_command, node_shell_manifest,
};

use super::pods::Pods;
use super::wait::Container;
use guard::PodCleanup;
use heartbeat::now_secs;
pub(super) use live::LiveShells;

/// A shell on a node.
#[derive(Debug)]
pub struct NodeShellSession {
    /// The shell pod's namespace.
    pub namespace: String,
    /// The shell pod's name.
    pub pod: String,
    /// The terminal session. Its `status` resolves when the shell exits, after the pod was
    /// deleted; dropping the session deletes the pod in the background.
    pub session: ExecSession,
}

/// Creates the pod, waits for it, opens the shell. Every failure deletes the pod first.
pub(super) async fn open(
    exec: &dyn ExecStreamPort,
    pods: Arc<dyn Pods>,
    live: &Arc<LiveShells>,
    spec: &NodeShellSpec,
) -> OxiResult<NodeShellSession> {
    let manifest = node_shell_manifest(spec)?;
    let namespace = spec.namespace.clone();
    let node = spec.node.as_str();
    let pod = pods.create(&namespace, &manifest).await?;
    tracing::info!(node, namespace = %namespace, pod = %pod, "node shell pod created");
    let cleanup = PodCleanup::new(pods.clone(), live, &namespace, &pod);

    let started = async {
        pods.wait_running(
            &namespace,
            &pod,
            &Container::Regular(NODE_SHELL_CONTAINER.into()),
            spec.start_timeout,
        )
        .await?;
        let options = ExecOptions::interactive().container(NODE_SHELL_CONTAINER);
        exec.exec_session(&namespace, &pod, &node_shell_command(spec), &options)
            .await
    }
    .await;
    let session = match started {
        Ok(session) => session,
        Err(err) => {
            cleanup.run().await;
            return Err(err);
        }
    };

    let ExecSession {
        stdin,
        stdout,
        stderr,
        resize,
        status,
    } = session;
    // The cleanup rides on the status future: it runs when the shell ends, and when the
    // session is dropped the guard's drop does it instead.
    let status = Box::pin(async move {
        let outcome = status.await;
        cleanup.run().await;
        outcome
    });
    Ok(NodeShellSession {
        namespace,
        pod,
        session: ExecSession {
            stdin,
            stdout,
            stderr,
            resize,
            status,
        },
    })
}

/// Deletes the node-shell pods of `namespace` that nobody has stamped alive for `older_than`
/// (its newest heartbeat or its creation): a pod whose shell is open keeps its stamp fresh however long
/// it has lived, so it is another window's or user's live shell that is spared, and only a pod
/// whose owner stopped is deleted. A pod that fails to delete is skipped and logged. Returns the
/// names of the pods that were deleted.
pub(super) async fn sweep(
    pods: &dyn Pods,
    namespace: &str,
    older_than: Duration,
) -> OxiResult<Vec<String>> {
    let cutoff =
        now_secs()?.saturating_sub(i64::try_from(older_than.as_secs()).unwrap_or(i64::MAX));
    let mut deleted = Vec::new();
    for pod in pods.list(namespace, NODE_SHELL_LABEL).await? {
        if pod.last_seen() > cutoff {
            continue;
        }
        match pods.delete(namespace, &pod.name).await {
            Ok(()) => deleted.push(pod.name),
            Err(err) => tracing::warn!(
                namespace, pod = %pod.name, error = %err,
                "could not delete a leftover node shell pod"
            ),
        }
    }
    Ok(deleted)
}
