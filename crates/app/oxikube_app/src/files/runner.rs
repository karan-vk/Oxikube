//! [`FileRunner`]: runs `pod::DownloadFile` and `pod::UploadFile` through the bus for a view.

use std::path::Path;
use std::sync::Arc;

use oxikube_domain::audit::Initiator;
use oxikube_domain::command::Command;
use oxikube_domain::ids::ResourceRef;
use oxikube_domain::safety::ConfirmTier;
use oxikube_domain::{OxiError, OxiResult};

use super::service::FileService;
use super::upload::UploadPlan;
use crate::command_bus::{CommandBus, DispatchContext, Outcome};
use crate::guard::Confirmation;

/// Dispatches the file commands on the bus as the user. Cheap to clone. The view calls these on
/// the Tokio bridge (`oxikube_runtime::spawn_kube`): each waits for its transfer to end.
///
/// An upload is a guarded mutation: the read-only block, the confirmation tier and the audit
/// record all apply. The view has already asked the user (the file picker or the drop is the
/// intent; replacing a file is a separate question it asks first, from
/// [`plan_upload`](Self::plan_upload)), so this answers the guard's simple confirmation once,
/// as the debug dialog's runner does. It is not a way around the guard.
#[derive(Clone)]
pub struct FileRunner {
    bus: CommandBus,
    who: Arc<str>,
    service: Arc<FileService>,
}

impl std::fmt::Debug for FileRunner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileRunner").finish_non_exhaustive()
    }
}

impl FileRunner {
    /// A runner dispatching on `bus` as `who` (the local user's name, for the audit log).
    pub fn new(bus: CommandBus, who: impl Into<Arc<str>>, service: Arc<FileService>) -> Self {
        Self {
            bus,
            who: who.into(),
            service,
        }
    }

    /// The service the commands run on: the listing, the board.
    pub fn service(&self) -> &Arc<FileService> {
        &self.service
    }

    /// What uploading `local` into `dest_dir` would do. See [`FileService::plan_upload`].
    ///
    /// # Errors
    ///
    /// As [`FileService::plan_upload`].
    pub async fn plan_upload(
        &self,
        pod: &ResourceRef,
        container: Option<&str>,
        dest_dir: &str,
        local: &Path,
    ) -> OxiResult<UploadPlan> {
        self.service
            .plan_upload(pod, container, dest_dir, local)
            .await
    }

    /// Uploads the planned file; `overwrite` says the user agreed to replace an existing one.
    ///
    /// # Errors
    ///
    /// `Forbidden` for a read-only cluster, `Conflict` when the file exists and `overwrite` is
    /// off, and the transfer's own errors.
    pub async fn upload(
        &self,
        pod: &ResourceRef,
        container: Option<&str>,
        dest_dir: &str,
        plan: &UploadPlan,
        overwrite: bool,
    ) -> OxiResult<String> {
        self.run(Command::PodUploadFile {
            target: pod.clone(),
            container: container.map(str::to_owned),
            dest_dir: dest_dir.to_owned(),
            local_path: plan.local.to_string_lossy().into_owned(),
            size: plan.size,
            overwrite: overwrite && plan.exists,
        })
        .await
    }

    /// Runs `command` (a `pod::DownloadFile`), returning the line to show.
    ///
    /// # Errors
    ///
    /// The transfer's own errors (a cancel is a `Conflict`).
    pub async fn download(&self, command: Command) -> OxiResult<String> {
        self.run(command).await
    }

    async fn run(&self, command: Command) -> OxiResult<String> {
        let cluster = command
            .target()
            .map(|target| target.cluster.clone())
            .ok_or_else(|| OxiError::validation("the command names no pod"))?;
        let context =
            || DispatchContext::new(Initiator::Ui, self.who.clone()).with_cluster(cluster.clone());
        let outcome = self.bus.dispatch(command.clone(), context()).await?;
        let outcome = match outcome {
            Outcome::NeedsConfirmation(asked) => {
                if asked.tier > ConfirmTier::Simple {
                    self.bus.decline(asked.token).await?;
                    return Err(OxiError::conflict(
                        "the confirmation this needs changed; try again",
                    ));
                }
                self.bus
                    .dispatch(
                        command,
                        context().with_confirmation(Confirmation::simple(asked.token)),
                    )
                    .await?
            }
            done @ Outcome::Completed(_) => done,
        };
        let Outcome::Completed(output) = outcome else {
            return Err(OxiError::conflict("the confirmation was not accepted"));
        };
        Ok(output.message.unwrap_or_else(|| "done".to_owned()))
    }
}
