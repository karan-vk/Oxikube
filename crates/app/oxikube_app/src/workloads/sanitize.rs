//! Sweeping Completed and Failed pods (E12-S03, k9s's `z`) and the per-object batch runner other
//! bulk actions reuse (E12-S19).
//!
//! `pod::Sanitize` is one guarded command (read-only check, a confirmation naming the count, one
//! audit record for the command). Inside it [`WorkloadService::delete_each`] deletes the pods one
//! by one: each is re-read (a pod that started again since the view drew it is left alone), is
//! rehearsed on the server, deleted, and gets its own audit record carrying the same
//! `initiator` and a shared `batch=<id>`, so the trail shows what happened to every pod and which
//! sweep it belonged to.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use futures::StreamExt as _;
use futures::stream;
use oxikube_domain::audit::AuditOutcome;
use oxikube_domain::command::CommandId;
use oxikube_domain::ids::{ClusterId, Gvk, ResourceRef};
use oxikube_domain::view::{PodPhase, PodSummary};
use oxikube_domain::{ErrorKind, OxiError, OxiResult};
use oxikube_ports::{DeleteOptions, ListOptions, Preconditions, PropagationPolicy};

use super::service::WorkloadService;
use crate::command_bus::HandlerContext;
use crate::guard::Mutation;

/// How many pods are deleted at once.
const CONCURRENCY: usize = 6;

/// How many pods of one sweep: a view never lists more than this, and neither does a sweep.
pub const MAX_SWEEP: usize = 5_000;

/// What happened to one pod of a batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PodStatus {
    /// Deleted (or, for a dry run, would be).
    Deleted,
    /// Left alone, and why (it is gone, or no longer Completed or Failed).
    Skipped(String),
    /// The delete was refused or failed, as the server said.
    Failed(String),
}

/// One pod of a batch and what happened to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PodResult {
    /// The pod.
    pub target: ResourceRef,
    /// What happened.
    pub status: PodStatus,
}

/// The result of a batch: one entry per pod, in the order asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sweep {
    /// The id every audit record of the batch carries (`batch=<id>`).
    pub batch: String,
    /// Whether this was a dry run.
    pub dry_run: bool,
    /// One result per pod.
    pub pods: Vec<PodResult>,
}

impl Sweep {
    /// How many pods were deleted.
    pub fn deleted(&self) -> usize {
        self.count(|s| matches!(s, PodStatus::Deleted))
    }

    /// How many pods were left alone.
    pub fn skipped(&self) -> usize {
        self.count(|s| matches!(s, PodStatus::Skipped(_)))
    }

    /// How many deletes failed.
    pub fn failed(&self) -> usize {
        self.count(|s| matches!(s, PodStatus::Failed(_)))
    }

    fn count(&self, pick: impl Fn(&PodStatus) -> bool) -> usize {
        self.pods.iter().filter(|p| pick(&p.status)).count()
    }

    /// One line: `Deleted 4 pods, 1 skipped`, or `Dry run: 4 pods can be deleted`.
    pub fn summary(&self) -> String {
        let (deleted, skipped, failed) = (self.deleted(), self.skipped(), self.failed());
        let pods = |n: usize| format!("{n} pod{}", if n == 1 { "" } else { "s" });
        let mut line = if self.dry_run {
            format!("Dry run: {} can be deleted", pods(deleted))
        } else {
            format!("Deleted {}", pods(deleted))
        };
        if skipped > 0 {
            line.push_str(&format!(", {skipped} skipped"));
        }
        if failed > 0 {
            line.push_str(&format!(", {failed} failed"));
        }
        line
    }

    /// The failures, as `pod: reason` lines.
    pub fn failures(&self) -> Vec<String> {
        self.pods
            .iter()
            .filter_map(|p| match &p.status {
                PodStatus::Failed(reason) => Some(format!("{}: {reason}", p.target.name)),
                _ => None,
            })
            .collect()
    }
}

/// A new id for a batch: unique within the process and across restarts, short enough to read in
/// an audit row.
pub(super) fn new_batch_id() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("sweep-{:x}-{n:x}", nanos & 0xffff_ffff_ffff)
}

impl WorkloadService {
    /// The Completed (`Succeeded`) and `Failed` pods of `namespace` (every namespace when
    /// `None`), listed with server-side field selectors so a quiet pod is never sent to us.
    ///
    /// # Errors
    ///
    /// `Conflict` when the cluster is not connected, the server's error for the list, and
    /// `Validation` past [`MAX_SWEEP`] pods.
    pub async fn finished_pods(
        &self,
        cluster: &ClusterId,
        namespace: Option<&str>,
    ) -> OxiResult<Vec<ResourceRef>> {
        let reader = self.reader(cluster)?;
        let gvk = Gvk::new("", "v1", "Pod");
        let mut out = Vec::new();
        for phase in ["Succeeded", "Failed"] {
            let mut options = ListOptions::default()
                .fields(format!("status.phase={phase}"))
                .limit(500);
            loop {
                let page = reader.list(&gvk, namespace, &options).await?;
                out.extend(page.items.iter().map(|pod| {
                    ResourceRef::new(
                        cluster.clone(),
                        gvk.clone(),
                        pod.namespace().map(Arc::from),
                        Arc::from(pod.name()),
                    )
                }));
                if out.len() > MAX_SWEEP {
                    return Err(OxiError::validation(format!(
                        "more than {MAX_SWEEP} Completed and Failed pods: narrow the namespace"
                    )));
                }
                match page.continue_token {
                    Some(token) => options.continue_token = Some(token),
                    None => break,
                }
            }
        }
        Ok(out)
    }

    /// Deletes each pod of `pods` that is still Completed or Failed, as `pod::Sanitize` does, and
    /// reports every one.
    ///
    /// For each pod: it is read again (gone: skipped; running or pending again: skipped, so a view
    /// that went stale never deletes a live pod); the server rehearses the delete; the delete is
    /// sent unless `mutation` is a dry run (both carry the UID of the pod that was read as a
    /// precondition, so a namesake created since is skipped, never deleted); and the pod gets its own audit record (`pod::Delete`,
    /// the command's initiator, `batch=<id> via=pod::Sanitize`, `dry_run` as dispatched). A
    /// failure on one pod does not stop the others.
    ///
    /// # Errors
    ///
    /// Only when the audit log cannot be written (the sweep stops before the next delete, fail
    /// closed); a pod's own failure is its [`PodStatus::Failed`].
    pub async fn delete_each(
        &self,
        cx: &HandlerContext,
        mutation: &Mutation,
        pods: Vec<ResourceRef>,
        batch: &str,
    ) -> OxiResult<Sweep> {
        let results: Vec<OxiResult<PodResult>> = stream::iter(pods)
            .map(|pod| async move {
                let status = self.sweep_one(cx, mutation, &pod, batch).await?;
                Ok(PodResult {
                    target: pod,
                    status,
                })
            })
            .buffered(CONCURRENCY)
            .collect()
            .await;
        let pods = results.into_iter().collect::<OxiResult<Vec<_>>>()?;
        Ok(Sweep {
            batch: batch.to_owned(),
            dry_run: mutation.dry_run(),
            pods,
        })
    }

    /// One pod of a sweep.
    async fn sweep_one(
        &self,
        cx: &HandlerContext,
        mutation: &Mutation,
        pod: &ResourceRef,
        batch: &str,
    ) -> OxiResult<PodStatus> {
        let reader = self.reader(&pod.cluster)?;
        let live = match reader.get_opt(&pod.gvk, pod.namespace(), &pod.name).await {
            Ok(Some(live)) => live,
            Ok(None) => return Ok(PodStatus::Skipped("already gone".to_owned())),
            Err(error) => return Ok(PodStatus::Failed(error.message().to_owned())),
        };
        let phase =
            PodSummary::from_resource(&live).map_or(PodPhase::Unknown, |summary| summary.phase);
        if !phase.is_terminal() {
            return Ok(PodStatus::Skipped(format!(
                "it is {} now, not Completed or Failed",
                phase.as_str()
            )));
        }
        let audit = cx
            .audit()
            .ok_or_else(|| OxiError::internal("a sweep needs the guard's audit log"))?;
        // Fail closed: a pod is deleted only when its record can be written.
        audit.ensure_writable().await?;
        let attempt = audit.begin_described(
            cx.who(),
            cx.initiator(),
            CommandId::POD_DELETE.as_str(),
            pod.clone(),
            mutation.dry_run(),
            Some(&format!("batch={batch} via={}", CommandId::POD_SANITIZE)),
        );
        // Only the pod that was read is deleted: a namesake created since (a StatefulSet pod comes
        // back under the same name) has another UID, and the server refuses with a conflict.
        let options = DeleteOptions {
            propagation: Some(PropagationPolicy::Background),
            preconditions: live.meta.uid.as_deref().map(|uid| Preconditions {
                uid: Some(uid.to_owned()),
                resource_version: None,
            }),
            ..DeleteOptions::default()
        };
        let writer = mutation.writer();
        let (namespace, name) = (pod.namespace(), &*pod.name);
        let rehearsal = DeleteOptions {
            dry_run: true,
            ..options.clone()
        };
        let mut result = writer.delete(&pod.gvk, namespace, name, &rehearsal).await;
        if result.is_ok() && !mutation.dry_run() {
            result = writer.delete(&pod.gvk, namespace, name, &options).await;
        }
        let (status, outcome) = match result {
            Ok(_) => (PodStatus::Deleted, AuditOutcome::Succeeded),
            Err(error) if super::pods::is_gone(&error) => (
                PodStatus::Skipped("already gone".to_owned()),
                AuditOutcome::Succeeded,
            ),
            Err(error) if error.kind() == ErrorKind::Conflict => (
                PodStatus::Skipped("replaced by a new pod since it was read".to_owned()),
                AuditOutcome::Succeeded,
            ),
            Err(error) => (
                PodStatus::Failed(error.message().to_owned()),
                AuditOutcome::Failed,
            ),
        };
        attempt.finish(outcome);
        audit.flush().await?;
        Ok(status)
    }
}
