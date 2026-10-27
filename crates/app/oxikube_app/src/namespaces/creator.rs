//! [`NamespaceCreator`]: what the create dialog runs.

use std::collections::BTreeMap;
use std::sync::Arc;

use oxikube_domain::audit::Initiator;
use oxikube_domain::command::Command;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::safety::ConfirmTier;
use oxikube_domain::{OxiError, OxiResult};

use super::input::{validate_labels, validate_name};
use crate::CommandBus;
use crate::command_bus::{DispatchContext, Outcome};
use crate::guard::Confirmation;

/// Dispatches `namespace::Create` on the bus as the user, answering the guard's one-click
/// confirmation on behalf of the dialog that was the confirmation (the dialog names the cluster
/// and the namespace). Anything stricter than that is a person's to answer, not the dialog's.
///
/// Cheap to clone. Plain async Rust: the caller spawns [`create`](Self::create) on the Tokio
/// bridge (`oxikube_runtime::spawn_kube`).
#[derive(Clone)]
pub struct NamespaceCreator {
    bus: CommandBus,
    who: Arc<str>,
}

impl std::fmt::Debug for NamespaceCreator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NamespaceCreator").finish_non_exhaustive()
    }
}

impl NamespaceCreator {
    /// A creator dispatching on `bus` as `who` (the local user's name, for the audit log).
    pub fn new(bus: CommandBus, who: impl Into<Arc<str>>) -> Self {
        Self {
            bus,
            who: who.into(),
        }
    }

    /// Creates the namespace `name` with `labels` in `cluster` and returns the handler's
    /// message ("Created namespace team-a").
    ///
    /// # Errors
    ///
    /// `Validation` for a bad name or label (nothing is sent), `Forbidden` for a read-only
    /// cluster or an account that may not create namespaces, `Conflict` for a name that exists,
    /// and the API server's own refusal otherwise.
    pub async fn create(
        &self,
        cluster: &ClusterId,
        name: &str,
        labels: BTreeMap<String, String>,
    ) -> OxiResult<String> {
        validate_name(name)?;
        validate_labels(&labels).map_err(|error| OxiError::validation(error.to_string()))?;
        let command = Command::NamespaceCreate {
            cluster: cluster.clone(),
            name: name.to_owned(),
            labels,
        };
        let context =
            || DispatchContext::new(Initiator::Ui, self.who.clone()).with_cluster(cluster.clone());
        let outcome = match self.bus.dispatch(command.clone(), context()).await? {
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
        Ok(output
            .message
            .unwrap_or_else(|| format!("Created namespace {name}")))
    }
}
