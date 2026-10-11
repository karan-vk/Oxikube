//! The rollout half of [`WorkloadService`]: restart (`kubectl rollout restart`) and set image
//! (`kubectl set image`) for Deployments, StatefulSets and DaemonSets (E12-S05).
//!
//! Both are patches of the workload's pod template, so the controller rolls new pods; the
//! service returns as soon as the server accepted the patch and never waits for the rollout (the
//! detail's Ready / Up-to-date / Available counts follow it live).

use oxikube_domain::build::{
    ImageEdit, plan_image_changes, restart_patch, set_image_patch, template_images,
};
use oxikube_domain::command::ImageChange;
use oxikube_domain::ids::ResourceRef;
use oxikube_domain::view::WorkloadKind;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::Patch;

use super::rollout_plan::{RestartResult, SetImagePlan, SetImageResult};
use super::service::{WorkloadService, options};
use crate::actions::object_label;
use crate::guard::Mutation;

impl WorkloadService {
    /// What a set-image dialog for `target` starts with: its containers and their images, read
    /// from the live object. Reads only.
    ///
    /// # Errors
    ///
    /// `Validation` for a kind without a rolling pod template, `NotFound` for an unknown
    /// workload, `Forbidden` for an account that may not read it, `Conflict` for a cluster that
    /// is not connected.
    pub async fn plan_set_image(&self, target: &ResourceRef) -> OxiResult<SetImagePlan> {
        let kind = check_rollable(target, "have its image set")?;
        let live = self.read(target).await?;
        Ok(SetImagePlan {
            target: target.clone(),
            kind,
            containers: template_images(&live)?,
        })
    }

    /// Restarts `target`: merge-patches `spec.template.metadata.annotations` with
    /// `kubectl.kubernetes.io/restartedAt` set to the session clock's time, which changes the pod
    /// template and so makes the controller roll new pods. Runs inside the guarded
    /// `workload::Restart` command: `mutation` is the guard's permit for this cluster.
    ///
    /// The server rehearses the patch first (admission, RBAC) unless the dispatch is a dry run,
    /// which stops there. `note` gets `restartedAt=<time> rv=<resourceVersion>` once the server
    /// answered. Returns without waiting for the rollout.
    ///
    /// # Errors
    ///
    /// `Validation` for a kind that cannot be restarted (a ReplicaSet, a Pod), `NotFound` for an
    /// unknown workload, `Forbidden` for a read-only cluster that went read-only meanwhile or an
    /// account without the `patch` right, and the server's own message for an admission refusal.
    pub async fn restart(
        &self,
        mutation: &Mutation,
        target: &ResourceRef,
        note: impl Fn(String),
    ) -> OxiResult<RestartResult> {
        check_permit(mutation, target)?;
        check_rollable(target, "be restarted")?;
        let at = self.sessions.now();
        let patch = Patch::merge(restart_patch(at));
        let patched = self.send(mutation, target, &patch).await?;
        let result = RestartResult {
            target: target.clone(),
            restarted_at: at,
            resource_version: patched
                .meta
                .resource_version
                .as_ref()
                .map(ToString::to_string),
            dry_run: mutation.dry_run(),
        };
        note(restart_note(&result));
        tracing::info!(
            workload = %target.name,
            dry_run = mutation.dry_run(),
            "workload restarted"
        );
        Ok(result)
    }

    /// Sets the images of `changes` on `target`'s pod template with a strategic merge patch that
    /// names only those containers (init containers included), so every other container and
    /// field is left alone. Runs inside the guarded `workload::SetImage` command.
    ///
    /// Reads the live template first: to find out which list each container is in, to drop a
    /// change to the image a container runs already, and to put the old image in the audit note
    /// (`web=nginx:1.26->nginx:1.27 rv=42`, set after the server answered).
    ///
    /// # Errors
    ///
    /// `Validation` for a kind without a rolling pod template, an empty list, an empty image or
    /// one with a space, a container the template does not have or a list that changes nothing;
    /// the same errors as [`restart`](Self::restart) otherwise.
    pub async fn set_image(
        &self,
        mutation: &Mutation,
        target: &ResourceRef,
        changes: &[ImageChange],
        note: impl Fn(String),
    ) -> OxiResult<SetImageResult> {
        check_permit(mutation, target)?;
        check_rollable(target, "have its image set")?;
        let live = self.read(target).await?;
        let edits: Vec<ImageEdit> = plan_image_changes(&template_images(&live)?, changes)?;
        let patch = Patch::strategic(set_image_patch(&edits));
        let patched = self.send(mutation, target, &patch).await?;
        let result = SetImageResult {
            target: target.clone(),
            edits,
            resource_version: patched
                .meta
                .resource_version
                .as_ref()
                .map(ToString::to_string),
            dry_run: mutation.dry_run(),
        };
        note(result.audit_note());
        tracing::info!(
            workload = %target.name,
            containers = result.edits.len(),
            dry_run = mutation.dry_run(),
            "workload image set"
        );
        Ok(result)
    }

    /// Sends `patch`: a server dry run first (admission, quota and RBAC are rehearsed before
    /// anything changes), then the real request, unless the dispatch is itself a dry run.
    async fn send(
        &self,
        mutation: &Mutation,
        target: &ResourceRef,
        patch: &Patch,
    ) -> OxiResult<oxikube_domain::Resource> {
        let writer = mutation.writer();
        let (namespace, name) = (target.namespace(), &*target.name);
        if !mutation.dry_run() {
            writer
                .patch(&target.gvk, namespace, name, patch, &options(true))
                .await?;
        }
        writer
            .patch(
                &target.gvk,
                namespace,
                name,
                patch,
                &options(mutation.dry_run()),
            )
            .await
    }
}

/// `restartedAt=2026-10-06T12:34:56Z rv=42`.
fn restart_note(result: &RestartResult) -> String {
    let at = result.stamp();
    match &result.resource_version {
        Some(version) => format!("restartedAt={at} rv={version}"),
        None => format!("restartedAt={at}"),
    }
}

/// The controller kind of `target` when a rollout action applies to it; `what` completes
/// "cannot ..." in the refusal.
fn check_rollable(target: &ResourceRef, what: &str) -> OxiResult<WorkloadKind> {
    match WorkloadKind::of(&target.gvk) {
        Some(
            kind @ (WorkloadKind::Deployment | WorkloadKind::StatefulSet | WorkloadKind::DaemonSet),
        ) => Ok(kind),
        _ => Err(OxiError::validation(format!(
            "{} cannot {what}: only Deployments, StatefulSets and DaemonSets can.",
            object_label(target)
        ))),
    }
}

/// The permit must be for the workload's cluster: a handler never writes elsewhere.
fn check_permit(mutation: &Mutation, target: &ResourceRef) -> OxiResult<()> {
    if mutation.cluster() == &target.cluster {
        Ok(())
    } else {
        Err(OxiError::internal(
            "the guard's permit is for another cluster than the workload's",
        ))
    }
}
