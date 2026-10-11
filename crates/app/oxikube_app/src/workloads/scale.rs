//! The scale half of [`WorkloadService`]: what a scale dialog starts with, and the scale itself.

use std::sync::Arc;

use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_domain::view::WorkloadKind;
use oxikube_domain::{ErrorKind, OxiError, OxiResult};
use oxikube_ports::{ListOptions, ResourceReader};

use super::plan::{ScalePlan, ScaleResult, is_scalable};
use super::service::WorkloadService;
use crate::guard::Mutation;

impl WorkloadService {
    /// What a scale dialog for `target` starts with: the `scale` subresource's replicas, and the
    /// HorizontalPodAutoscaler that targets the workload, if one is found. Reads only.
    ///
    /// # Errors
    ///
    /// `Validation` for a kind that cannot be scaled, `NotFound` for an unknown workload,
    /// `Forbidden` for an account that may not read it, `Conflict` for a cluster that is not
    /// connected. The autoscaler lookup never fails the plan.
    pub async fn plan_scale(&self, target: &ResourceRef) -> OxiResult<ScalePlan> {
        let kind = check_scalable(&target.gvk)?;
        let reader = self.reader(&target.cluster)?;
        let scale = reader
            .get_scale(&target.gvk, target.namespace(), &target.name)
            .await?;
        let autoscaler = autoscaler_of(&reader, target).await;
        Ok(ScalePlan {
            target: target.clone(),
            kind,
            current: non_negative(scale.current_replicas),
            desired: non_negative(scale.replicas),
            autoscaler,
        })
    }

    /// Sets the replicas of `target` through its `scale` subresource. Runs inside the guarded
    /// `workload::Scale` command: `mutation` is the guard's permit for this cluster, and the only
    /// way to a writer.
    ///
    /// Reads the subresource first for the audit record's `before=` (a read the account may lack:
    /// then the scale goes ahead and the record says `before=unknown`), writes `spec.replicas`
    /// with a merge patch of the subresource, and notes `after=`. A dry-run permit has the server
    /// rehearse the patch and changes nothing.
    ///
    /// # Errors
    ///
    /// `Validation` for a kind that cannot be scaled (a DaemonSet) or a count beyond the API's
    /// range, `NotFound` for an unknown workload, `Forbidden` for a read-only cluster that went
    /// read-only meanwhile or an account without the `scale` right, and the server's own message
    /// for an admission refusal.
    pub async fn scale(
        &self,
        mutation: &Mutation,
        target: &ResourceRef,
        replicas: u32,
        note: impl Fn(String),
    ) -> OxiResult<ScaleResult> {
        if mutation.cluster() != &target.cluster {
            return Err(OxiError::internal(
                "the guard's permit is for another cluster than the workload's",
            ));
        }
        check_scalable(&target.gvk)?;
        let wanted = i32::try_from(replicas)
            .map_err(|_| OxiError::validation("Replicas cannot be more than 2147483647."))?;
        let before = match self
            .reader(&target.cluster)?
            .get_scale(&target.gvk, target.namespace(), &target.name)
            .await
        {
            Ok(scale) => Some(non_negative(scale.replicas)),
            Err(error) if error.kind() == ErrorKind::NotFound => return Err(error),
            Err(error) => {
                tracing::debug!(kind = ?error.kind(), "the scale could not be read first");
                None
            }
        };
        let was = before.map_or_else(|| "unknown".to_owned(), |n| n.to_string());
        // Set before the write: a refused or failed scale still records what it started from.
        note(format!("before={was}"));
        let scale = mutation
            .writer()
            .scale(
                &target.gvk,
                target.namespace(),
                &target.name,
                wanted,
                &mutation.write_options(),
            )
            .await?;
        let after = non_negative(scale.replicas);
        note(format!("before={was} after={after}"));
        tracing::info!(
            workload = %target.name,
            replicas = after,
            dry_run = mutation.dry_run(),
            "workload scaled"
        );
        Ok(ScaleResult {
            target: target.clone(),
            before,
            after,
            dry_run: mutation.dry_run(),
        })
    }
}

/// The controller kind of `gvk`, when it can be scaled.
fn check_scalable(gvk: &Gvk) -> OxiResult<WorkloadKind> {
    match WorkloadKind::of(gvk) {
        Some(WorkloadKind::DaemonSet) => Err(OxiError::validation(
            "A DaemonSet runs one pod per node and cannot be scaled.",
        )),
        Some(kind) if is_scalable(gvk) => Ok(kind),
        _ => Err(OxiError::validation(format!(
            "{} cannot be scaled: only Deployments, StatefulSets, ReplicaSets and \
             ReplicationControllers have a scale.",
            gvk.kind
        ))),
    }
}

fn non_negative(n: i32) -> u32 {
    u32::try_from(n).unwrap_or(0)
}

/// The name of a HorizontalPodAutoscaler in `target`'s namespace whose `scaleTargetRef` names
/// it. `None` when there is none, or the list failed (no `list` right, no autoscaling API): a
/// hint, never an error.
async fn autoscaler_of(reader: &Arc<dyn ResourceReader>, target: &ResourceRef) -> Option<Arc<str>> {
    let hpa = Gvk::new("autoscaling", "v2", "HorizontalPodAutoscaler");
    let page = reader
        .list(&hpa, target.namespace(), &ListOptions::default())
        .await
        .ok()?;
    page.items.iter().find_map(|autoscaler| {
        let scale_target = autoscaler.json().pointer("/spec/scaleTargetRef")?;
        let kind = scale_target.get("kind")?.as_str()?;
        let name = scale_target.get("name")?.as_str()?;
        (kind == &*target.gvk.kind && name == &*target.name).then(|| autoscaler.meta.name.clone())
    })
}
