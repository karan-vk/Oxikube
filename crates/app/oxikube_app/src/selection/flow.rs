//! [`SelectionFlow`]: what a bulk dialog asks (the plan) and the run that answers the guard on its
//! behalf.

use std::sync::Arc;

use oxikube_domain::OxiError;
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::{self, Command};
use oxikube_domain::ids::ResourceRef;
use oxikube_domain::safety::{ConfirmTier, Risk};
use oxikube_domain::selection::SelectionSummary;

use super::report::BulkReport;
use crate::command_bus::{CommandBus, DispatchContext, Outcome};
use crate::guard::{Confirmation, policy};
use crate::session::ClusterSessionManager;

/// Why a bulk command was not planned or not run.
#[derive(Debug, thiserror::Error)]
pub enum SelectionError {
    /// Nothing was selected.
    #[error("nothing is selected")]
    Empty,
    /// The objects belong to more than one cluster; a bulk action acts on one cluster.
    #[error("the selected objects belong to more than one cluster")]
    MixedClusters,
    /// The text the user typed is not the one the confirmation asks for. Nothing was sent.
    #[error("type {expected:?} to confirm")]
    NameMismatch {
        /// What the user has to type.
        expected: String,
    },
    /// The batch was refused or failed as a whole (read-only cluster, a kind that cannot be
    /// restarted, an invalid label): nothing was sent.
    #[error("{0}")]
    Refused(OxiError),
}

/// What a bulk command will ask of the user, worked out before anything is sent: the command, the
/// confirmation tier the guard will ask (the highest among the objects), and what has to be typed.
#[derive(Debug, Clone, PartialEq)]
pub struct SelectionPlan {
    command: Command,
    summary: SelectionSummary,
    tier: ConfirmTier,
    risk: Risk,
    context: String,
    phrase: Option<String>,
}

impl SelectionPlan {
    /// The command the run dispatches.
    pub fn command(&self) -> &Command {
        &self.command
    }

    /// The objects, in the order selected.
    pub fn targets(&self) -> &[ResourceRef] {
        self.command.targets()
    }

    /// How many objects of each kind in each namespace.
    pub fn summary(&self) -> &SelectionSummary {
        &self.summary
    }

    /// The highest confirmation tier among the objects: [`ConfirmTier::Simple`] or
    /// [`ConfirmTier::TypeName`].
    pub fn tier(&self) -> ConfirmTier {
        self.tier
    }

    /// The highest risk among the objects.
    pub fn risk(&self) -> Risk {
        self.risk
    }

    /// The cluster's context name, for the text of a dialog.
    pub fn context(&self) -> &str {
        &self.context
    }

    /// For [`ConfirmTier::TypeName`]: the text the user types. The cluster's context name: a
    /// selection has no single name, and typing every name does not scale.
    pub fn phrase(&self) -> Option<&str> {
        self.phrase.as_deref()
    }
}

/// Plans and runs bulk commands through the `CommandBus`. Cheap to clone; it holds no UI: a view
/// builds its dialog from a [`SelectionPlan`] and calls [`run`](Self::run) on the Tokio bridge
/// (`oxikube_runtime::spawn_kube`).
#[derive(Clone)]
pub struct SelectionFlow {
    bus: CommandBus,
    sessions: ClusterSessionManager,
    who: Arc<str>,
}

impl std::fmt::Debug for SelectionFlow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SelectionFlow").finish_non_exhaustive()
    }
}

impl SelectionFlow {
    /// A flow dispatching on `bus` as `who` (the local user's name, for the audit log).
    /// `sessions` supplies the cluster's context name for the typed confirmation.
    pub fn new(bus: CommandBus, sessions: ClusterSessionManager, who: impl Into<Arc<str>>) -> Self {
        Self {
            bus,
            sessions,
            who: who.into(),
        }
    }

    /// Plans `command` (`selection::Delete`, `selection::Restart` or `selection::Label`). Sends
    /// nothing and asks nobody.
    ///
    /// The tier is the guard's own ([`policy::confirm_tier_for`]), so a dialog drawn from the plan
    /// asks what the guard will ask.
    ///
    /// # Errors
    ///
    /// [`SelectionError::Empty`] for no objects, [`SelectionError::MixedClusters`] for objects of
    /// more than one cluster.
    ///
    /// # Panics
    ///
    /// When `command` is not declared in `oxikube_domain::command`.
    pub fn plan(&self, command: Command) -> Result<SelectionPlan, SelectionError> {
        let targets = command.targets();
        let first = targets.first().ok_or(SelectionError::Empty)?;
        let cluster = first.cluster.clone();
        if targets.iter().any(|t| t.cluster != cluster) {
            return Err(SelectionError::MixedClusters);
        }
        let meta = command::lookup(command.id()).expect("every command is declared");
        let tier = policy::confirm_tier_for(meta, &command);
        let context = self
            .sessions
            .get(&cluster)
            .map_or_else(|| cluster.to_string(), |s| s.context().to_string());
        Ok(SelectionPlan {
            summary: SelectionSummary::of(targets),
            tier,
            risk: command.effective_risk().unwrap_or(Risk::High),
            phrase: (tier == ConfirmTier::TypeName).then(|| context.clone()),
            context,
            command,
        })
    }

    /// Plans the same command over only `targets` (the rows that failed, for a retry).
    ///
    /// # Errors
    ///
    /// As [`plan`](Self::plan).
    pub fn plan_again(
        &self,
        plan: &SelectionPlan,
        targets: Vec<ResourceRef>,
    ) -> Result<SelectionPlan, SelectionError> {
        let command = match plan.command.clone() {
            Command::SelectionDelete { propagation, .. } => Command::SelectionDelete {
                targets,
                propagation,
            },
            Command::SelectionRestart { .. } => Command::SelectionRestart { targets },
            Command::SelectionLabel { field, edits, .. } => Command::SelectionLabel {
                targets,
                field,
                edits,
            },
            other => other,
        };
        self.plan(command)
    }

    /// Runs the plan's command and returns the result of every object.
    ///
    /// `typed` is what the user typed for a [`ConfirmTier::TypeName`] plan; it must equal
    /// [`SelectionPlan::phrase`] or nothing is sent. The command is one dispatch through the
    /// guard (read-only check for the whole batch, confirmation, audit); the guard answers it with
    /// a confirmation request, which this flow answers on behalf of the user's single confirmation
    /// in the dialog. A request for a tier above the plan's is declined: a person has not seen
    /// that dialog.
    ///
    /// The future is plain async Rust: the caller spawns it (`oxikube_runtime::spawn_kube`).
    ///
    /// # Errors
    ///
    /// [`SelectionError::NameMismatch`] when `typed` is not the phrase the plan asks for, and
    /// [`SelectionError::Refused`] when the batch as a whole was refused or failed.
    pub async fn run(
        &self,
        plan: &SelectionPlan,
        typed: Option<&str>,
    ) -> Result<BulkReport, SelectionError> {
        if let Some(expected) = plan.phrase()
            && typed != Some(expected)
        {
            return Err(SelectionError::NameMismatch {
                expected: expected.to_owned(),
            });
        }
        let first = plan.targets().first().ok_or(SelectionError::Empty)?;
        let context = || {
            DispatchContext::new(Initiator::Ui, self.who.clone())
                .with_cluster(first.cluster.clone())
        };
        let command = plan.command.clone();
        let refused = |error| SelectionError::Refused(OxiError::from(error));
        let outcome = self
            .bus
            .dispatch(command.clone(), context())
            .await
            .map_err(refused)?;
        let outcome = match outcome {
            Outcome::NeedsConfirmation(request) => {
                if request.tier > plan.tier {
                    self.bus.decline(request.token).await.map_err(refused)?;
                    return Err(SelectionError::Refused(OxiError::conflict(
                        "the confirmation this needs changed; try again",
                    )));
                }
                let answer = match (request.tier, request.expected_name) {
                    (ConfirmTier::TypeName, Some(name)) => Confirmation::typed(request.token, name),
                    _ => Confirmation::simple(request.token),
                };
                self.bus
                    .dispatch(command, context().with_confirmation(answer))
                    .await
                    .map_err(refused)?
            }
            done @ Outcome::Completed(_) => done,
        };
        let Outcome::Completed(output) = outcome else {
            return Err(SelectionError::Refused(OxiError::conflict(
                "the confirmation was not accepted",
            )));
        };
        BulkReport::from_data(output.data.as_ref()).map_err(SelectionError::Refused)
    }
}
