//! [`WorkloadService`]: the logic of the trigger, suspend and resume commands.

use std::sync::Arc;

use oxikube_domain::build::{job_from_cronjob, suspend_patch};
use oxikube_domain::ids::{ClusterId, Gvk, ResourceRef};
use oxikube_domain::{JobSummary, OxiError, OxiResult, Resource};
use oxikube_ports::{Patch, ResourceReader, WriteOptions};

use crate::actions::object_label;
use crate::apply::DEFAULT_FIELD_MANAGER;
use crate::guard::Mutation;
use crate::session::ClusterSessionManager;

/// What `cronjob::Trigger` did.
#[derive(Debug, Clone, PartialEq)]
pub struct Triggered {
    /// The Job the server made (or, for a dry run, would make).
    pub job: ResourceRef,
    /// Whether this was a dry run: nothing was created.
    pub dry_run: bool,
}

/// What a suspend or a resume did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuspendOutcome {
    /// The object.
    pub target: ResourceRef,
    /// Its `spec.suspend` afterwards.
    pub suspended: bool,
    /// Whether this was a dry run: nothing changed.
    pub dry_run: bool,
}

/// Trigger, suspend and resume, over a cluster's reader and the guard's [`Mutation`].
///
/// Cheap to clone. It reads through the session's reader (the CronJob to copy, the Job to check)
/// and writes only through the [`Mutation`] a handler got from the guard, so it cannot be used to
/// bypass read-only mode, the confirmation or the audit.
#[derive(Clone, Debug)]
pub struct WorkloadService {
    sessions: ClusterSessionManager,
}

impl WorkloadService {
    /// A service reading through `sessions`.
    pub fn new(sessions: ClusterSessionManager) -> Self {
        Self { sessions }
    }

    /// Runs `cronjob` now: creates a Job from its job template (`kubectl create job
    /// --from=cronjob/x`).
    ///
    /// The server rehearses the create first (admission, quota, RBAC); a dispatch that is a dry
    /// run stops there and returns the Job the server would make.
    ///
    /// # Errors
    ///
    /// `Validation` when `cronjob` is not a CronJob or has no job template or uid, `NotFound` when
    /// it is gone, `Conflict` when the cluster is not connected, and the server's error for the
    /// create (`Forbidden`, admission, quota).
    pub async fn trigger_cronjob(
        &self,
        mutation: &Mutation,
        cronjob: &ResourceRef,
    ) -> OxiResult<Triggered> {
        require_kind(cronjob, "CronJob")?;
        let live = self.read(cronjob).await?;
        let job = job_from_cronjob(&live)?;
        let namespace = cronjob.namespace();
        let job_gvk = Gvk::new("batch", "v1", "Job");
        let writer = mutation.writer();
        if !mutation.dry_run() {
            writer
                .create(&job_gvk, namespace, &job, &options(true))
                .await?;
        }
        let created = writer
            .create(&job_gvk, namespace, &job, &options(mutation.dry_run()))
            .await?;
        Ok(Triggered {
            job: ResourceRef::new(
                cronjob.cluster.clone(),
                job_gvk,
                created.namespace().map(Arc::from),
                Arc::from(created.name()),
            ),
            dry_run: mutation.dry_run(),
        })
    }

    /// Suspends a CronJob (it creates no more Jobs) or a Job (its controller deletes its running
    /// pods until it is resumed): a merge patch of `spec.suspend: true`.
    ///
    /// # Errors
    ///
    /// `Validation` when `target` is neither a CronJob nor a Job, and the server's error for the
    /// patch.
    pub async fn suspend(
        &self,
        mutation: &Mutation,
        target: &ResourceRef,
    ) -> OxiResult<SuspendOutcome> {
        self.set_suspend(mutation, target, true).await
    }

    /// Resumes a suspended CronJob or Job: a merge patch of `spec.suspend: false`.
    ///
    /// A Job that finished (`Complete` or `Failed`) cannot be resumed: the server refuses it, and
    /// the user is told so before anything is sent.
    ///
    /// # Errors
    ///
    /// `Validation` when `target` is neither a CronJob nor a Job, or is a finished Job, and the
    /// server's error for the patch.
    pub async fn resume(
        &self,
        mutation: &Mutation,
        target: &ResourceRef,
    ) -> OxiResult<SuspendOutcome> {
        self.set_suspend(mutation, target, false).await
    }

    async fn set_suspend(
        &self,
        mutation: &Mutation,
        target: &ResourceRef,
        suspend: bool,
    ) -> OxiResult<SuspendOutcome> {
        let job = is_kind(target, "Job");
        if !job && !is_kind(target, "CronJob") {
            return Err(OxiError::validation(format!(
                "{} cannot be suspended: only CronJobs and Jobs can",
                object_label(target)
            )));
        }
        if job && !suspend {
            let live = self.read(target).await?;
            if JobSummary::from_resource(&live).is_ok_and(|summary| summary.is_finished()) {
                return Err(OxiError::validation(format!(
                    "{} has finished and cannot be resumed",
                    object_label(target)
                )));
            }
        }
        let patch = Patch::merge(suspend_patch(suspend));
        let writer = mutation.writer();
        let (namespace, name) = (target.namespace(), &*target.name);
        if !mutation.dry_run() {
            writer
                .patch(&target.gvk, namespace, name, &patch, &options(true))
                .await?;
        }
        writer
            .patch(
                &target.gvk,
                namespace,
                name,
                &patch,
                &options(mutation.dry_run()),
            )
            .await?;
        Ok(SuspendOutcome {
            target: target.clone(),
            suspended: suspend,
            dry_run: mutation.dry_run(),
        })
    }

    /// The live object `target`.
    async fn read(&self, target: &ResourceRef) -> OxiResult<Resource> {
        let reader = self.reader(&target.cluster)?;
        reader
            .get(&target.gvk, target.namespace(), &target.name)
            .await
    }

    fn reader(&self, cluster: &ClusterId) -> OxiResult<Arc<dyn ResourceReader>> {
        self.sessions
            .get(cluster)
            .ok_or_else(|| OxiError::not_found("the cluster is not open"))?
            .resources()
            .ok_or_else(|| OxiError::conflict("the cluster is not connected"))
    }
}

/// The write options of these commands: the app's field manager, and a dry run when asked.
fn options(dry_run: bool) -> WriteOptions {
    WriteOptions {
        dry_run,
        ..WriteOptions::default().manager(DEFAULT_FIELD_MANAGER)
    }
}

fn is_kind(target: &ResourceRef, kind: &str) -> bool {
    &*target.gvk.group == "batch" && &*target.gvk.kind == kind
}

fn require_kind(target: &ResourceRef, kind: &str) -> OxiResult<()> {
    if is_kind(target, kind) {
        Ok(())
    } else {
        Err(OxiError::validation(format!(
            "{} is not a {kind}",
            object_label(target)
        )))
    }
}
