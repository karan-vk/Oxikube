//! The leftover sweep: deleting the shell pods whose owner is gone, audited.

use std::time::Duration;

use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::CommandId;
use oxikube_domain::ids::{ClusterId, Gvk, ResourceRef};
use oxikube_ports::ExecPort;

use crate::exec::service::ExecService;
use crate::guard::policy::node_shell_sweep_detail;

/// How long nothing may have stamped a node shell pod alive (its creation, or the heartbeat its
/// owner refreshes every minute while the shell is open) before the leftover sweep deletes it.
/// A shell that is open in another window, process or machine keeps stamping, so it is spared
/// however long it lives (the pod's own deadline defaults to eight hours); a pod this quiet has
/// an owner that crashed, quit or lost its connection, and would otherwise stay privileged until
/// its deadline. Many times the heartbeat interval, so a laptop that napped or a slow API server
/// never makes a live shell look abandoned.
pub const JANITOR_GRACE: Duration = Duration::from_secs(15 * 60);

/// The longest the sweep may take before the shell opens anyway.
const SWEEP_TIMEOUT: Duration = Duration::from_secs(10);

impl ExecService {
    /// Deletes the shell pods an earlier run left in `namespace`, once per cluster and namespace
    /// per run, and audits each deletion (`node::Shell`, `phase=sweep`) as the user whose shell
    /// is opening: the sweep only runs inside a guarded `node::Shell`, so it is never done on a
    /// read-only cluster. Best effort: a failure (no `list` on pods there) never stops the shell.
    ///
    /// A sweep that times out is not audited: the pods it deleted before are only in the log.
    pub(super) async fn sweep_once(
        &self,
        cluster: &ClusterId,
        port: &dyn ExecPort,
        namespace: &str,
        who: &str,
        initiator: Initiator,
    ) {
        if !self
            .swept
            .lock()
            .insert((cluster.clone(), namespace.to_owned()))
        {
            return;
        }
        let swept = match tokio::time::timeout(
            SWEEP_TIMEOUT,
            port.sweep_node_shells(namespace, JANITOR_GRACE),
        )
        .await
        {
            Ok(Ok(swept)) => swept,
            Ok(Err(error)) => {
                tracing::debug!(
                    kind = ?error.kind(),
                    namespace,
                    "could not sweep leftover node shell pods"
                );
                return;
            }
            Err(_) => {
                tracing::warn!(
                    namespace,
                    "sweeping leftover node shell pods timed out; any it deleted are unaudited"
                );
                return;
            }
        };
        if swept.is_empty() {
            return;
        }
        tracing::info!(
            count = swept.len(),
            namespace,
            "removed leftover node shell pods"
        );
        let Some(log) = self.audit.get() else {
            return;
        };
        let detail = node_shell_sweep_detail(namespace);
        for pod in swept {
            let target =
                ResourceRef::namespaced(cluster.clone(), Gvk::new("", "v1", "Pod"), namespace, pod);
            let record = log.entry_with_detail(
                who,
                initiator,
                CommandId::NODE_SHELL.as_str(),
                target,
                &detail,
                AuditOutcome::Succeeded,
            );
            if let Err(error) = log.record(record).await {
                tracing::warn!(kind = ?error.kind(), "could not audit a swept node shell pod");
            }
        }
    }
}
