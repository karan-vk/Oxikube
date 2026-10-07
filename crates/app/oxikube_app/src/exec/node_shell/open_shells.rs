//! The node shells that are open now, so the app's quit can end them.

use std::sync::{Arc, Weak};

use futures::future::join_all;
use oxikube_ports::ExecPort;
use parking_lot::Mutex;

use super::backend::CloseAudit;
use crate::audit::AuditLog;

/// What [`ExecService`](crate::ExecService) remembers of its node shells: the exec ports that
/// opened them (one adapter deletes its own pods) and the closing audit of each.
///
/// GPUI does not drop the terminals when the app quits, so without this the privileged pods of
/// the open tabs would stay until their deadline and their deletions would never be audited.
#[derive(Default)]
pub(in crate::exec) struct OpenShells {
    ports: Mutex<Vec<Arc<dyn ExecPort>>>,
    audits: Mutex<Vec<Weak<CloseAudit>>>,
}

impl OpenShells {
    /// Remembers a shell opened through `port`, with its closing audit if the app writes one.
    pub(super) fn track(&self, port: &Arc<dyn ExecPort>, close: Option<&Arc<CloseAudit>>) {
        {
            let mut ports = self.ports.lock();
            if !ports.iter().any(|known| Arc::ptr_eq(known, port)) {
                ports.push(port.clone());
            }
        }
        let mut audits = self.audits.lock();
        // Shells that closed already have nothing left to write; forget them.
        audits.retain(|audit| audit.upgrade().is_some_and(|audit| !audit.is_written()));
        if let Some(close) = close {
            audits.push(Arc::downgrade(close));
        }
    }

    /// Ends every open shell: has each port delete its pods (waiting for the answers), then
    /// writes the closing record of every shell that had not written one and flushes the audit
    /// log, which also writes the records the tabs closed earlier left queued. Returns how many
    /// pods were deleted. Safe to call again: the second call finds nothing open.
    pub(in crate::exec) async fn close_all(&self, log: Option<&Arc<AuditLog>>) -> usize {
        let ports: Vec<_> = self.ports.lock().clone();
        let outcomes = join_all(ports.iter().map(|port| port.release_node_shells())).await;
        let mut deleted = 0;
        for outcome in outcomes {
            match outcome {
                Ok(count) => deleted += count,
                Err(error) => tracing::warn!(
                    kind = ?error.kind(),
                    "could not delete the open node shell pods on quit; their deadline will"
                ),
            }
        }
        let audits: Vec<_> = std::mem::take(&mut *self.audits.lock())
            .into_iter()
            .filter_map(|audit| audit.upgrade())
            .collect();
        for audit in &audits {
            audit.queue();
        }
        if let Some(log) = log
            && let Err(error) = log.flush().await
        {
            tracing::warn!(kind = ?error.kind(), "could not audit the end of the open node shells");
        }
        deleted
    }
}
