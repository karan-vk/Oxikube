//! `ApplyService::plan`, `dry_run_plan` and `apply_plan`: a plan of many objects through one guarded
//! `resource::ApplyPlan` dispatch each.

use std::sync::Arc;

use futures::FutureExt as _;
use futures::channel::mpsc;
use oxikube_domain::apply::ManifestObjects;
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::Command;
use oxikube_domain::manifests::ManifestDoc;
use oxikube_domain::safety::ConfirmTier;

use super::build::build_plan;
use super::stream::PlanStream;
use super::types::{ApplyOptions, ClusterPlan, PlanTarget};
use crate::apply::service::ApplyService;
use crate::apply::types::ApplyError;
use crate::command_bus::{DispatchContext, DispatchError, Outcome, Progress};
use crate::guard::{Confirmation, ConfirmationProvider};

impl ApplyService {
    /// Orders `docs` for `target` (E20-S02's [`ApplyPlan`](oxikube_domain::apply::ApplyPlan):
    /// kind rank, namespace defaulting, `depends_on` edges) after sanitising each for apply (the
    /// server's fields go, what the user wrote stays). Whether a kind is namespaced comes from the
    /// cluster's discovery. Nothing is sent but discovery lookups.
    pub async fn plan(&self, docs: Vec<ManifestDoc>, target: PlanTarget) -> ClusterPlan {
        let discovery = self
            .sessions
            .get(&target.cluster)
            .and_then(|session| session.discovery());
        let plan = build_plan(docs, target.namespace.as_deref(), discovery.as_deref()).await;
        ClusterPlan { target, plan }
    }

    /// The server's dry run of every object of `plan`, in plan order, streamed as each is decided:
    /// what applying it would do ([`Created`](super::ObjectStatus::Created),
    /// [`Configured`](super::ObjectStatus::Configured), ...), or
    /// [`DeferredDryRun`](super::ObjectStatus::DeferredDryRun) for an object that needs one the
    /// plan creates first. Nothing is written. Each object is audited as a dry run.
    ///
    /// The guard's confirmation of the dry run itself is answered here: the user asked for this
    /// rehearsal, and it changes nothing. A read-only cluster refuses it before any request.
    pub fn dry_run_plan(&self, plan: &ClusterPlan, options: &ApplyOptions) -> PlanStream {
        self.run(plan, options, Answer::DryRun)
    }

    /// Applies `plan`: one guarded `resource::ApplyPlan` dispatch, so one confirmation for the
    /// whole plan, asked of `confirm` at the riskiest object's tier (the dry run's results are
    /// what the user reviews; [`dry_run_plan`](Self::dry_run_plan) first). The handler rehearses the whole
    /// plan again, then applies it rank by rank, and every object is audited under one batch id.
    /// Results stream as each object is done.
    ///
    /// A read-only cluster, a declined confirmation or an invalid plan is one
    /// [`ApplyError`] item and nothing is sent.
    pub fn apply_plan(
        &self,
        plan: &ClusterPlan,
        options: &ApplyOptions,
        confirm: Arc<dyn ConfirmationProvider>,
    ) -> PlanStream {
        self.run(plan, options, Answer::Provider(confirm))
    }

    fn run(&self, plan: &ClusterPlan, options: &ApplyOptions, answer: Answer) -> PlanStream {
        let command = Command::ResourceApplyPlan {
            cluster: plan.target.cluster.clone(),
            namespace: plan.target.namespace.clone(),
            objects: ManifestObjects(
                plan.plan
                    .items
                    .iter()
                    .map(|item| item.doc.json.clone())
                    .collect(),
            ),
            field_manager: options.field_manager.clone(),
            force_conflicts: options.force_conflicts,
            stop_on_error: options.stop_on_error,
        };
        let (sender, reports) = mpsc::unbounded();
        let progress = Progress::new(
            move |report| {
                // The receiver is gone only when the stream was dropped, which stops the run.
                let _ = sender.unbounded_send(report);
            },
            options.cancel.clone(),
        );
        let context = DispatchContext::new(Initiator::Ui, self.who.clone())
            .with_cluster(plan.target.cluster.clone())
            .with_dry_run(matches!(answer, Answer::DryRun))
            .with_progress(progress);
        let service = self.clone();
        let run = async move { service.dispatch_plan(command, context, answer).await };
        PlanStream::new(run.boxed(), reports)
    }

    async fn dispatch_plan(
        &self,
        command: Command,
        context: DispatchContext,
        answer: Answer,
    ) -> Result<(), ApplyError> {
        let outcome = match answer {
            Answer::Provider(provider) => {
                self.bus.execute(command, context, provider.as_ref()).await
            }
            Answer::DryRun => match self.bus.dispatch(command.clone(), context.clone()).await {
                Ok(Outcome::NeedsConfirmation(asked)) => {
                    if asked.tier > ConfirmTier::Simple {
                        let _ = self.bus.decline(asked.token).await;
                        return Err(ApplyError::Unconfirmed);
                    }
                    let answer = Confirmation::simple(asked.token);
                    self.bus
                        .dispatch(command, context.with_confirmation(answer))
                        .await
                }
                other => other,
            },
        };
        match outcome {
            Ok(Outcome::Completed(_)) => Ok(()),
            Ok(Outcome::NeedsConfirmation(_)) => Err(ApplyError::Unconfirmed),
            Err(error) => Err(plan_error(error)),
        }
    }
}

/// Who answers the guard's confirmation.
enum Answer {
    /// The dry run's own, answered here.
    DryRun,
    /// The real apply's, asked of the user (or the agent's permission prompt).
    Provider(Arc<dyn ConfirmationProvider>),
}

/// The [`ApplyError`] for a dispatch the guard or the handler refused as a whole.
fn plan_error(error: DispatchError) -> ApplyError {
    match error {
        DispatchError::ReadOnly { context, .. } => ApplyError::ReadOnly {
            context: context.to_string(),
        },
        DispatchError::Declined(_) => ApplyError::Declined,
        other => ApplyError::Failed(oxikube_domain::OxiError::from(other).to_string()),
    }
}
