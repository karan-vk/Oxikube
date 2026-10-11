//! [`SetImageRunner`]: runs `workload::SetImage` through the bus for a dialog that already asked.

use std::sync::Arc;

use oxikube_domain::OxiResult;
use oxikube_domain::command::{Command, ImageChange};
use oxikube_domain::ids::ResourceRef;

use super::runner::run_confirmed;
use crate::command_bus::CommandBus;

/// What a finished `workload::SetImage` says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetImageReport {
    /// The line to show the user.
    pub message: String,
}

/// Dispatches `workload::SetImage` on the bus as the user, answering the guard's confirmation on
/// the set-image dialog's behalf. Cheap to clone. The view calls [`run`](Self::run) on the Tokio
/// bridge (`oxikube_runtime::spawn_kube`).
///
/// The dialog is the user's confirmation (it names the workload and shows every old and new image
/// before its button), so this answers the guard's simple confirmation once. It is not a way
/// around it: the command still passes the read-only check, the server dry run and the audit
/// record, as `Initiator::Ui`.
#[derive(Clone)]
pub struct SetImageRunner {
    bus: CommandBus,
    who: Arc<str>,
}

impl std::fmt::Debug for SetImageRunner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SetImageRunner").finish_non_exhaustive()
    }
}

impl SetImageRunner {
    /// A runner dispatching on `bus` as `who` (the local user's name, for the audit log).
    pub fn new(bus: CommandBus, who: impl Into<Arc<str>>) -> Self {
        Self {
            bus,
            who: who.into(),
        }
    }

    /// Sets the images of `changes` on `target`.
    ///
    /// # Errors
    ///
    /// `Forbidden` for a read-only cluster or an account without the `patch` right, `Validation`
    /// for a kind without a rolling pod template or a change the template cannot take, and the
    /// handler's own error otherwise (the API server's message for an admission refusal).
    pub async fn run(
        &self,
        target: &ResourceRef,
        changes: Vec<ImageChange>,
    ) -> OxiResult<SetImageReport> {
        let command = Command::WorkloadSetImage {
            target: target.clone(),
            images: changes,
        };
        let output = run_confirmed(&self.bus, &self.who, command).await?;
        Ok(SetImageReport {
            message: output
                .message
                .unwrap_or_else(|| "the image was set".to_owned()),
        })
    }
}
