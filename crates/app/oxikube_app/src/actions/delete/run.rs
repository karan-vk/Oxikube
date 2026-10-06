//! [`DeleteFlow::run`]: delete every planned object through the bus, each on its own.

use futures::StreamExt as _;
use futures::stream::FuturesOrdered;
use oxikube_domain::ErrorKind;
use oxikube_domain::OxiError;
use oxikube_domain::audit::Initiator;
use oxikube_domain::ids::ResourceRef;
use oxikube_domain::safety::ConfirmTier;

use super::plan::{DeleteError, DeleteFlow, DeletePlan, PlannedDelete};
use crate::command_bus::{DispatchContext, DispatchError, Outcome};
use crate::guard::Confirmation;

/// How many objects are deleted at once.
pub(super) const CONCURRENCY: usize = 4;

/// What happened to one object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemStatus {
    /// The object is gone.
    Deleted,
    /// Deletion started but the object is still there (finalizers, foreground propagation).
    Deleting,
    /// Refused: read-only mode, or the user may not delete it.
    Forbidden(String),
    /// The object no longer exists.
    NotFound(String),
    /// Anything else, with the error's message.
    Failed(String),
}

impl ItemStatus {
    /// Whether the delete went through (deleted or on its way).
    pub fn is_success(&self) -> bool {
        matches!(self, ItemStatus::Deleted | ItemStatus::Deleting)
    }

    /// A short word for a results list.
    pub fn label(&self) -> &'static str {
        match self {
            ItemStatus::Deleted => "Deleted",
            ItemStatus::Deleting => "Deleting",
            ItemStatus::Forbidden(_) => "Forbidden",
            ItemStatus::NotFound(_) => "Not found",
            ItemStatus::Failed(_) => "Failed",
        }
    }

    /// The error's message, for a refusal or a failure.
    pub fn message(&self) -> Option<&str> {
        match self {
            ItemStatus::Forbidden(m) | ItemStatus::NotFound(m) | ItemStatus::Failed(m) => Some(m),
            ItemStatus::Deleted | ItemStatus::Deleting => None,
        }
    }

    fn from_error(error: DispatchError) -> Self {
        let error = OxiError::from(error);
        let message = error.message().to_owned();
        match error.kind() {
            ErrorKind::Forbidden => ItemStatus::Forbidden(message),
            ErrorKind::NotFound => ItemStatus::NotFound(message),
            _ => ItemStatus::Failed(message),
        }
    }
}

/// The result for one object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemResult {
    /// The object.
    pub target: ResourceRef,
    /// What happened.
    pub status: ItemStatus,
}

/// Every object's result, in the order they were selected.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DeleteReport {
    /// One result per object.
    pub items: Vec<ItemResult>,
}

impl DeleteReport {
    /// How many deletes went through.
    pub fn succeeded(&self) -> usize {
        self.items.iter().filter(|i| i.status.is_success()).count()
    }

    /// How many did not.
    pub fn failed(&self) -> usize {
        self.items.len() - self.succeeded()
    }

    /// One line: "Deleted 3 objects" or "Deleted 3 of 5 objects, 2 failed".
    pub fn summary(&self) -> String {
        let total = self.items.len();
        let noun = if total == 1 { "object" } else { "objects" };
        match self.failed() {
            0 => format!("Deleted {total} {noun}"),
            failed => format!(
                "Deleted {} of {total} {noun}, {failed} failed",
                self.succeeded()
            ),
        }
    }
}

impl DeleteFlow {
    /// Deletes every object of `plan`, as the UI, and reports each result.
    ///
    /// `typed` is what the user typed for a [`ConfirmTier::TypeName`] plan; it must equal
    /// [`DeletePlan::phrase`] or nothing is sent. Every object is its own `resource::Delete`
    /// dispatch through the guard (read-only check, confirmation, dry run, delete, one audit
    /// record with [`Initiator::Ui`]): the guard answers each with a confirmation request, which
    /// this flow answers on behalf of the user's single confirmation (for a typed plan with the
    /// request's own name, once the phrase matched). One failure does not stop the rest; a few
    /// objects are deleted at a time and the results keep the selection's order.
    ///
    /// The future is plain async Rust: the caller spawns it (`oxikube_runtime::spawn_kube`).
    ///
    /// # Errors
    ///
    /// [`DeleteError::NameMismatch`] when `typed` is not the phrase the plan asks for.
    pub async fn run(
        &self,
        plan: &DeletePlan,
        typed: Option<&str>,
    ) -> Result<DeleteReport, DeleteError> {
        if let Some(expected) = plan.phrase()
            && typed != Some(expected)
        {
            return Err(DeleteError::NameMismatch {
                expected: expected.to_owned(),
            });
        }
        // A sliding window of `concurrency` deletes; `FuturesOrdered` yields in selection order.
        let mut running = FuturesOrdered::new();
        let mut waiting = plan.items().iter();
        let mut items = Vec::with_capacity(plan.items().len());
        loop {
            while running.len() < self.concurrency {
                let Some(item) = waiting.next() else { break };
                running.push_back(self.delete_one(item));
            }
            match running.next().await {
                Some(result) => items.push(result),
                None => break,
            }
        }
        Ok(DeleteReport { items })
    }

    async fn delete_one(&self, item: &PlannedDelete) -> ItemResult {
        let status = match self.dispatch(item).await {
            Ok(Outcome::Completed(output)) => match output
                .data
                .as_ref()
                .and_then(|d| d.get("outcome"))
                .and_then(|o| o.as_str())
            {
                Some("deleting") => ItemStatus::Deleting,
                _ => ItemStatus::Deleted,
            },
            Ok(Outcome::NeedsConfirmation(_)) => {
                ItemStatus::Failed("the confirmation was not accepted".to_owned())
            }
            Err(error) => ItemStatus::from_error(error),
        };
        ItemResult {
            target: item.target.clone(),
            status,
        }
    }

    async fn dispatch(&self, item: &PlannedDelete) -> Result<Outcome, DispatchError> {
        let context = || {
            DispatchContext::new(Initiator::Ui, self.who.clone())
                .with_cluster(item.target.cluster.clone())
        };
        let outcome = self.bus.dispatch(item.command.clone(), context()).await?;
        let Outcome::NeedsConfirmation(request) = outcome else {
            return Ok(outcome);
        };
        // The plan came from the guard's own policy; a tier above it means the policy moved
        // under us, and a person has not seen that dialog.
        if request.tier > item.tier {
            self.bus.decline(request.token).await?;
            return Err(DispatchError::Handler(OxiError::conflict(
                "the confirmation this delete needs changed; try again",
            )));
        }
        let answer = match (request.tier, request.expected_name) {
            (ConfirmTier::TypeName, Some(name)) => Confirmation::typed(request.token, name),
            _ => Confirmation::simple(request.token),
        };
        self.bus
            .dispatch(item.command.clone(), context().with_confirmation(answer))
            .await
    }
}
