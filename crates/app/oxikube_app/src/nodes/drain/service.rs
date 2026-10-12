//! [`NodeService`]: a guarded drain, from the `node::Drain` handler to its audit trail.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

use futures::StreamExt as _;
use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::CommandId;
use oxikube_domain::drain::{DrainFlags, DrainProgress, DrainSummary, PodRef};
use oxikube_domain::ids::ResourceRef;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{ClockPort, DrainPort};

use super::monitor::DrainMonitor;
use crate::audit::AuditLog;
use crate::guard::Mutation;
use crate::session::ClusterSessionManager;

/// How many failed pods an error message names.
const NAMED: usize = 5;

/// What a drain did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrainReport {
    /// The id the drain's audit record and its per-pod records share (`batch=` in each).
    pub batch: String,
    /// What the algorithm reported last.
    pub summary: DrainSummary,
}

/// Who asked for a drain, for the per-pod audit records.
#[derive(Debug, Clone)]
pub(crate) struct Requester {
    pub who: Arc<str>,
    pub initiator: Initiator,
}

/// Node operations that run longer than one request: the drain (E12-S10). Holds the
/// [`DrainPort`] (the kube adapter's `kubectl drain` algorithm), the sessions it reads through,
/// the [`DrainMonitor`] the dialog follows, and the guard's audit log for the per-pod records.
/// One per app; the bus's `node::Drain` handler and the drain dialog's flow share it.
pub struct NodeService {
    port: Arc<dyn DrainPort>,
    sessions: ClusterSessionManager,
    clock: Arc<dyn ClockPort>,
    monitor: DrainMonitor,
    audit: OnceLock<Arc<AuditLog>>,
    seq: AtomicU64,
}

impl std::fmt::Debug for NodeService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NodeService")
            .field("monitor", &self.monitor)
            .finish_non_exhaustive()
    }
}

impl NodeService {
    /// A service draining through `port`, reading through `sessions`, stamping batch ids with
    /// `clock`.
    pub fn new(
        port: Arc<dyn DrainPort>,
        sessions: ClusterSessionManager,
        clock: Arc<dyn ClockPort>,
    ) -> Self {
        Self {
            port,
            sessions,
            clock,
            monitor: DrainMonitor::new(),
            audit: OnceLock::new(),
            seq: AtomicU64::new(0),
        }
    }

    /// Makes the service write one audit record per evicted pod to `audit`, the
    /// [`MutationGuard`](crate::MutationGuard)'s log
    /// ([`audit_handle`](crate::MutationGuard::audit_handle)). Set once, after the bus is built;
    /// `false` when it was set already. Until it is set a drain is refused (fail closed).
    pub fn set_audit(&self, audit: Arc<AuditLog>) -> bool {
        self.audit.set(audit).is_ok()
    }

    /// The running drains and their followers.
    pub fn monitor(&self) -> &DrainMonitor {
        &self.monitor
    }

    /// The sessions the service reads through.
    pub(super) fn sessions(&self) -> &ClusterSessionManager {
        &self.sessions
    }

    /// `drain-<node>-<ms>-<n>`: unique within this process run.
    fn batch_id(&self, node: &str) -> String {
        let n = self.seq.fetch_add(1, Ordering::Relaxed);
        format!("drain-{node}-{}-{n}", self.clock.now().as_millisecond())
    }

    /// Drains `node` with `flags` through the guard's `mutation`: the handler of `node::Drain`.
    /// `note` receives the audit words of the drain's own record (the batch id first, the counts
    /// at the end).
    ///
    /// Every step goes to the [`DrainMonitor`]'s followers. Each pod that is gone (or failed)
    /// gets its own audit record (`node::Drain` on the pod, `batch=<id> node=<node>`), written
    /// before the next step is taken: a record that cannot be written stops the drain (fail
    /// closed). Dropping the future stops the drain; the node stays cordoned.
    ///
    /// # Errors
    ///
    /// `Validation` for a target that is not a node or flags out of range; `Conflict` while the
    /// node is being drained already, or when pods were left (named, up to five); `NotFound`
    /// when the cluster is not connected; the drain's own error otherwise (a pod blocks it,
    /// the node is missing, the cordon is refused, read-only went on mid-way).
    pub(crate) async fn drain(
        &self,
        mutation: &Mutation,
        requester: &Requester,
        node: &ResourceRef,
        flags: &DrainFlags,
        note: impl Fn(String),
    ) -> OxiResult<DrainReport> {
        if !node.gvk.is_node() || node.namespace.is_some() {
            return Err(OxiError::validation(format!(
                "drain needs a node, not a {}",
                node.gvk.kind
            )));
        }
        flags.validate()?;
        let audit = self
            .audit
            .get()
            .cloned()
            .ok_or_else(|| OxiError::internal("the drain has no audit log"))?;
        let reader = self
            .sessions
            .get(mutation.cluster())
            .and_then(|session| session.resources())
            .ok_or_else(|| OxiError::not_found("the cluster is not connected"))?;
        let publisher = self.monitor.start(mutation.cluster(), &node.name)?;
        let batch = self.batch_id(&node.name);
        note(format!("batch={batch}"));

        let mut options = flags.options();
        options.dry_run = mutation.dry_run();
        let mut steps = self
            .port
            .drain(reader, mutation.writer_handle(), &node.name, options);
        let mut summary = None;
        while let Some(step) = steps.next().await {
            let step = step?;
            if let Some((pod, outcome)) = settled(&step) {
                let record = audit.entry_with_detail(
                    &requester.who,
                    requester.initiator,
                    CommandId::NODE_DRAIN.as_str(),
                    pod.resource_ref(mutation.cluster()),
                    &format!("batch={batch} node={}", node.name),
                    outcome,
                );
                // Fail closed: no further eviction without its record.
                audit.record(record).await?;
            }
            publisher.publish(&step);
            if let DrainProgress::Finished(done) = step {
                summary = Some(done);
            }
        }
        let summary =
            summary.ok_or_else(|| OxiError::internal("the drain ended without a result"))?;
        note(format!(
            "batch={batch} evicted={} failed={} skipped={}",
            summary.evicted.len(),
            summary.failed.len(),
            summary.skipped.len()
        ));
        if !summary.is_complete() {
            return Err(OxiError::conflict(format!(
                "drain of node {} left {} pod(s): {}",
                node.name,
                summary.failed.len(),
                names(&summary.failed)
            )));
        }
        Ok(DrainReport { batch, summary })
    }
}

/// The pod a step settles and how, for its audit record.
fn settled(step: &DrainProgress) -> Option<(&PodRef, AuditOutcome)> {
    match step {
        DrainProgress::Gone { pod } => Some((pod, AuditOutcome::Succeeded)),
        DrainProgress::PodFailed { pod, .. } => Some((pod, AuditOutcome::Failed)),
        _ => None,
    }
}

/// `a/x, b/y` for the first few pods, `, ...` when there are more.
fn names(pods: &[PodRef]) -> String {
    let mut text = pods
        .iter()
        .take(NAMED)
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    if pods.len() > NAMED {
        text.push_str(", ...");
    }
    text
}
