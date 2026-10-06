//! Node shells: a shell on a node through a privileged pod (E04-S09).
//!
//! The pod ([`node_shell_manifest`]) is created on the target node, the shell is an exec of
//! `nsenter` into the node's namespaces, and the pod is deleted when the shell exits, when
//! opening fails at any step, and when the session is dropped or aborted (`guard`). If a run
//! dies before it can clean up (a crash, a lost connection), `sweep` deletes what it left,
//! and the pod's `activeDeadlineSeconds` ends it regardless.
//!
//! | Piece | Where |
//! |---|---|
//! | [`NodeShellConfig`] (image, pull secret, namespace, lifetime) | `config` |
//! | [`node_shell_manifest`], the exec command | `manifest` |
//! | cleanup on end, error and drop | `guard` |
//! | `open`, `sweep` | this file |

mod config;
mod guard;
mod manifest;

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{ExecOptions, ExecSession, ExecStreamPort};

use super::pods::Pods;
use super::wait::Container;
use guard::PodCleanup;
use manifest::{CONTAINER, exec_command};

pub use config::{DEFAULT_IMAGE, DEFAULT_NAMESPACE, NodeShellConfig};
pub use manifest::{NODE_ANNOTATION, NODE_SHELL_LABEL, node_shell_manifest};

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
    node: &str,
    config: &NodeShellConfig,
) -> OxiResult<NodeShellSession> {
    let manifest = node_shell_manifest(node, config)?;
    let namespace = config.namespace.clone();
    let pod = pods.create(&namespace, &manifest).await?;
    tracing::info!(node, namespace = %namespace, pod = %pod, "node shell pod created");
    let cleanup = PodCleanup::new(pods.clone(), &namespace, &pod);

    let started = async {
        pods.wait_running(
            &namespace,
            &pod,
            &Container::Regular(CONTAINER.into()),
            config.start_timeout,
        )
        .await?;
        let options = ExecOptions::interactive().container(CONTAINER);
        exec.exec_session(&namespace, &pod, &exec_command(config), &options)
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

/// Deletes the node-shell pods of `namespace` that are older than `older_than`. A pod younger
/// than that may be another window's live shell. A pod that fails to delete is skipped and
/// logged; the count is of deletions that went through.
pub(super) async fn sweep(
    pods: &dyn Pods,
    namespace: &str,
    older_than: Duration,
) -> OxiResult<usize> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| OxiError::internal("the system clock is before 1970"))?
        .as_secs();
    let cutoff = i64::try_from(now.saturating_sub(older_than.as_secs())).unwrap_or(i64::MAX);
    let mut deleted = 0;
    for pod in pods.list(namespace, NODE_SHELL_LABEL).await? {
        if pod.created > cutoff {
            continue;
        }
        match pods.delete(namespace, &pod.name).await {
            Ok(()) => deleted += 1,
            Err(err) => tracing::warn!(
                namespace, pod = %pod.name, error = %err,
                "could not delete a leftover node shell pod"
            ),
        }
    }
    Ok(deleted)
}
