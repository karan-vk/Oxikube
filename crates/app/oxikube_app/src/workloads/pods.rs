//! Evict, delete and force delete one pod (E12-S03): the logic of `pod::Evict`, `pod::Delete`
//! and `pod::ForceDelete`.

use oxikube_domain::ids::ResourceRef;
use oxikube_domain::{ErrorKind, OxiError, OxiResult};
use oxikube_ports::{DeleteOptions, DeleteOutcome, PropagationPolicy};

use super::service::WorkloadService;
use crate::actions::object_label;
use crate::guard::Mutation;

/// What an eviction did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Evicted {
    /// The pod.
    pub target: ResourceRef,
    /// Whether this was a dry run: nothing was evicted.
    pub dry_run: bool,
}

/// What a delete did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PodDeleted {
    /// The pod.
    pub target: ResourceRef,
    /// The grace period asked for (`Some(0)`: a force delete).
    pub grace_period_secs: Option<u32>,
    /// Whether the pod is still there, shutting down (it is, until the kubelet confirms).
    pub deleting: bool,
    /// Whether this was a dry run: nothing was deleted.
    pub dry_run: bool,
}

impl WorkloadService {
    /// Evicts `pod` through the eviction subresource, which honours PodDisruptionBudgets.
    ///
    /// The server rehearses the eviction first (`dryRun=All`): a budget that forbids it refuses
    /// the rehearsal, so nothing is evicted and the user is told which budget. A dispatch that is
    /// itself a dry run stops there.
    ///
    /// # Errors
    ///
    /// `Validation` when `pod` is not a Pod; `Conflict` (retryable) saying "blocked by disruption
    /// budget" with the budget's name when HTTP 429 refuses it; the server's error otherwise
    /// (`NotFound`, `Forbidden`).
    pub async fn evict(&self, mutation: &Mutation, pod: &ResourceRef) -> OxiResult<Evicted> {
        require_pod(pod)?;
        let namespace = pod
            .namespace()
            .ok_or_else(|| OxiError::validation("a pod has a namespace"))?;
        let writer = mutation.writer();
        let rehearsal = DeleteOptions::dry_run();
        writer
            .evict(namespace, &pod.name, &rehearsal)
            .await
            .map_err(|error| blocked_or(error, pod))?;
        if !mutation.dry_run() {
            writer
                .evict(namespace, &pod.name, &DeleteOptions::default())
                .await
                .map_err(|error| blocked_or(error, pod))?;
        }
        Ok(Evicted {
            target: pod.clone(),
            dry_run: mutation.dry_run(),
        })
    }

    /// Deletes `pod`: with its own grace period (`None`), another (`Some(30)`), or none
    /// (`Some(0)`, a force delete: the API object goes at once, whether or not the kubelet has
    /// stopped the containers).
    ///
    /// The server rehearses the delete first; a dispatch that is itself a dry run stops there.
    ///
    /// # Errors
    ///
    /// `Validation` when `pod` is not a Pod, and the server's error (`NotFound`, `Forbidden`,
    /// admission).
    pub async fn delete_pod(
        &self,
        mutation: &Mutation,
        pod: &ResourceRef,
        grace_period_secs: Option<u32>,
    ) -> OxiResult<PodDeleted> {
        require_pod(pod)?;
        let options = DeleteOptions {
            dry_run: false,
            propagation: Some(PropagationPolicy::Background),
            grace_period_secs,
            preconditions: None,
        };
        let writer = mutation.writer();
        let (namespace, name) = (pod.namespace(), &*pod.name);
        let rehearsal = DeleteOptions {
            dry_run: true,
            ..options.clone()
        };
        writer.delete(&pod.gvk, namespace, name, &rehearsal).await?;
        let mut deleting = false;
        if !mutation.dry_run() {
            deleting = matches!(
                writer.delete(&pod.gvk, namespace, name, &options).await?,
                DeleteOutcome::Deleting(_)
            );
        }
        Ok(PodDeleted {
            target: pod.clone(),
            grace_period_secs,
            deleting,
            dry_run: mutation.dry_run(),
        })
    }
}

/// Turns the adapter's PodDisruptionBudget refusal into the sentence a person acts on.
fn blocked_or(error: OxiError, pod: &ResourceRef) -> OxiError {
    let Some(blocked) = error.eviction_blocked() else {
        return error;
    };
    let budget = blocked.budget_name().map_or_else(
        || "a PodDisruptionBudget".to_owned(),
        |name| format!("disruption budget {name}"),
    );
    OxiError::conflict(format!(
        "{} cannot be evicted: blocked by {budget} ({})",
        object_label(pod),
        blocked.reason
    ))
    .with_retryable(true)
}

fn require_pod(target: &ResourceRef) -> OxiResult<()> {
    if target.gvk.is_pod() {
        Ok(())
    } else {
        Err(OxiError::validation(format!(
            "{} is not a Pod",
            object_label(target)
        )))
    }
}

/// Whether `error` is a pod that is already gone (a sweep counts it as done).
pub(super) fn is_gone(error: &OxiError) -> bool {
    error.kind() == ErrorKind::NotFound
}
