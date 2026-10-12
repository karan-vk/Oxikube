//! [`RolloutService`]: reading a workload's history, and rolling it back.

use std::sync::Arc;

use oxikube_domain::ids::{ClusterId, Gvk, ResourceRef};
use oxikube_domain::rollout::{
    PatchFlavor, PlanAction, RollbackPlan, RolloutKind, controller_history, deployment_history,
    label_selector, owned_controller_revisions, owned_replicasets, plan_controller_rollback,
    plan_deployment_rollback, refuse_if_paused, revision_of,
};
use oxikube_domain::{OxiError, OxiResult, Resource};
use oxikube_ports::{ListOptions, Patch, ResourceReader, WriteOptions};

use super::result::{RollbackOutcome, RollbackResult, RolloutHistory};
use crate::actions::object_label;
use crate::apply::DEFAULT_FIELD_MANAGER;
use crate::guard::Mutation;
use crate::session::ClusterSessionManager;

/// Reads rollout history and restores earlier revisions. Cheap to clone. See the
/// [module docs](super).
///
/// It reads through the session's reader and writes only through the [`Mutation`] a handler got
/// from the guard, so it cannot be used to bypass read-only mode, the confirmation or the audit.
#[derive(Clone, Debug)]
pub struct RolloutService {
    sessions: ClusterSessionManager,
}

/// A workload resolved from the target: the object, its kind, and the revision a ReplicaSet
/// target stands for.
struct Resolved {
    workload: Resource,
    kind: RolloutKind,
    reference: ResourceRef,
    of_replicaset: Option<i64>,
}

impl RolloutService {
    /// A service reading through `sessions`.
    pub fn new(sessions: ClusterSessionManager) -> Self {
        Self { sessions }
    }

    /// The rollout history of `target`: a Deployment, StatefulSet or DaemonSet, or a ReplicaSet
    /// (then its owning Deployment's, starting on that ReplicaSet's revision). Reads only.
    ///
    /// # Errors
    ///
    /// `Validation` for a kind with no history (or a ReplicaSet no Deployment owns), `NotFound`
    /// for an unknown workload, `Forbidden` for an account that may not list its ReplicaSets or
    /// ControllerRevisions, `Conflict` for a cluster that is not connected.
    pub async fn history(&self, target: &ResourceRef) -> OxiResult<RolloutHistory> {
        let reader = self.reader(&target.cluster)?;
        let resolved = resolve(&reader, target).await?;
        let owned = owned_objects(&reader, &resolved).await?;
        let revisions = match resolved.kind {
            RolloutKind::Deployment => deployment_history(&resolved.workload, &owned),
            _ => controller_history(&resolved.workload, &owned),
        };
        let previous = revisions.get(1).map(|r| r.revision);
        Ok(RolloutHistory {
            workload: resolved.reference,
            kind: resolved.kind,
            paused: resolved.workload.get_bool("/spec/paused") == Some(true),
            preselected: resolved.of_replicaset.or(previous),
            revisions,
        })
    }

    /// Restores `to_revision` of `target` (by default the previous one), as `kubectl rollout
    /// undo`. Runs inside the guarded `workload::Rollback` command: `mutation` is the guard's
    /// permit for this cluster, and the only way to a writer.
    ///
    /// The server rehearses the patch first; a dry-run permit stops there. A target that already
    /// runs the chosen template sends nothing. `note` receives the audit detail (`from=` and
    /// `to=`), set before the write so a refused rollback still records what it started from.
    ///
    /// # Errors
    ///
    /// `Validation` for a kind with no history, a revision that is not in it (pruned by
    /// `revisionHistoryLimit`) and a workload with no earlier revision; `Conflict` for a paused
    /// Deployment (kubectl's own message); `NotFound` for an unknown workload; `Forbidden` for a
    /// read-only cluster that went read-only meanwhile or an account without the `patch` right;
    /// the server's message for an admission refusal.
    pub async fn rollback(
        &self,
        mutation: &Mutation,
        target: &ResourceRef,
        to_revision: Option<i64>,
        note: impl Fn(String),
    ) -> OxiResult<RollbackResult> {
        if mutation.cluster() != &target.cluster {
            return Err(OxiError::internal(
                "the guard's permit is for another cluster than the workload's",
            ));
        }
        let reader = self.reader(&target.cluster)?;
        let resolved = resolve(&reader, target).await?;
        // A ReplicaSet names its own revision unless the caller chose another.
        let wanted = to_revision.or(resolved.of_replicaset);
        if resolved.kind == RolloutKind::Deployment {
            // Before listing the ReplicaSets: a paused Deployment costs one read.
            refuse_if_paused(&resolved.workload)?;
        }
        let owned = owned_objects(&reader, &resolved).await?;
        let plan = plan(&resolved, &owned, wanted)?;
        note(format!(
            "from={} to={}",
            plan.from_revision
                .map_or_else(|| "unknown".to_owned(), |from| from.to_string()),
            plan.to_revision
        ));
        let result = |outcome| RollbackResult {
            workload: resolved.reference.clone(),
            from_revision: plan.from_revision,
            to_revision: plan.to_revision,
            outcome,
        };
        let PlanAction::Patch { flavor, body } = &plan.action else {
            return Ok(result(RollbackOutcome::AlreadyCurrent));
        };
        let patch = match flavor {
            PatchFlavor::Json => Patch::json(body.clone()),
            PatchFlavor::Strategic => Patch::strategic(body.clone()),
        };
        let gvk = &resolved.reference.gvk;
        let (namespace, name) = (resolved.reference.namespace(), &*resolved.reference.name);
        let writer = mutation.writer();
        if !mutation.dry_run() {
            // The server checks the patch (admission, quota, RBAC) before the real one is sent.
            writer
                .patch(gvk, namespace, name, &patch, &options(true))
                .await?;
        }
        writer
            .patch(gvk, namespace, name, &patch, &options(mutation.dry_run()))
            .await?;
        tracing::info!(
            workload = %name,
            to_revision = plan.to_revision,
            dry_run = mutation.dry_run(),
            "workload rolled back"
        );
        Ok(result(if mutation.dry_run() {
            RollbackOutcome::DryRun
        } else {
            RollbackOutcome::RolledBack
        }))
    }

    fn reader(&self, cluster: &ClusterId) -> OxiResult<Arc<dyn ResourceReader>> {
        self.sessions
            .get(cluster)
            .ok_or_else(|| OxiError::not_found("the cluster is not open"))?
            .resources()
            .ok_or_else(|| OxiError::conflict("the cluster is not connected"))
    }
}

/// The write options of a rollback: the app's field manager, and a dry run when asked.
fn options(dry_run: bool) -> WriteOptions {
    WriteOptions {
        dry_run,
        ..WriteOptions::default().manager(DEFAULT_FIELD_MANAGER)
    }
}

/// The plan for `resolved` and `wanted`, by kind.
fn plan(
    resolved: &Resolved,
    owned: &[(i64, Resource)],
    wanted: Option<i64>,
) -> OxiResult<RollbackPlan> {
    match resolved.kind {
        RolloutKind::Deployment => plan_deployment_rollback(&resolved.workload, owned, wanted),
        kind => plan_controller_rollback(kind, &resolved.workload, owned, wanted),
    }
}

/// The workload `target` stands for, read from the cluster.
async fn resolve(reader: &Arc<dyn ResourceReader>, target: &ResourceRef) -> OxiResult<Resolved> {
    let namespace = target.namespace();
    if &*target.gvk.group == "apps" && &*target.gvk.kind == "ReplicaSet" {
        let replicaset = reader.get(&target.gvk, namespace, &target.name).await?;
        let owner = replicaset
            .meta
            .controller_ref()
            .filter(|owner| &*owner.kind == "Deployment")
            .ok_or_else(|| {
                OxiError::validation(format!(
                    "{} is not owned by a Deployment, so it has no rollout history",
                    object_label(target)
                ))
            })?;
        let gvk = Gvk::new("apps", "v1", "Deployment");
        let workload = reader.get(&gvk, namespace, &owner.name).await?;
        let reference = ResourceRef::new(
            target.cluster.clone(),
            gvk,
            target.namespace.clone(),
            owner.name.clone(),
        );
        return Ok(Resolved {
            workload,
            kind: RolloutKind::Deployment,
            reference,
            of_replicaset: revision_of(&replicaset),
        });
    }
    let kind = RolloutKind::of(&target.gvk).ok_or_else(|| {
        OxiError::validation(format!(
            "{} has no rollout history: only Deployments, StatefulSets and DaemonSets do",
            target.gvk.kind
        ))
    })?;
    let workload = reader.get(&target.gvk, namespace, &target.name).await?;
    Ok(Resolved {
        workload,
        kind,
        reference: target.clone(),
        of_replicaset: None,
    })
}

/// The ReplicaSets or ControllerRevisions the workload owns, oldest revision first.
async fn owned_objects(
    reader: &Arc<dyn ResourceReader>,
    resolved: &Resolved,
) -> OxiResult<Vec<(i64, Resource)>> {
    let workload = &resolved.workload;
    let options = ListOptions::default()
        .labels(label_selector(workload)?)
        .limit(500);
    let list = |kind| {
        list_all(
            reader,
            Gvk::new("apps", "v1", kind),
            workload.namespace(),
            options,
        )
    };
    Ok(match resolved.kind {
        RolloutKind::Deployment => owned_replicasets(workload, list("ReplicaSet").await?),
        _ => owned_controller_revisions(workload, list("ControllerRevision").await?),
    })
}

/// Every object `options` selects, following continue tokens to the last page.
async fn list_all(
    reader: &Arc<dyn ResourceReader>,
    gvk: Gvk,
    namespace: Option<&str>,
    mut options: ListOptions,
) -> OxiResult<Vec<Resource>> {
    let mut items = Vec::new();
    loop {
        let page = reader.list(&gvk, namespace, &options).await?;
        items.extend(page.items);
        match page.continue_token.filter(|token| !token.is_empty()) {
            Some(token) => options = options.continue_from(token),
            None => return Ok(items),
        }
    }
}
