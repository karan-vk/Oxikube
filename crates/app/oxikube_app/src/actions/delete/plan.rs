//! [`DeleteFlow::plan`]: what the guard will ask for a delete, before anything is sent.

use std::collections::BTreeMap;
use std::sync::Arc;

use oxikube_domain::command::{Command, CommandId, Propagation};
use oxikube_domain::ids::{ClusterId, ResourceRef};
use oxikube_domain::safety::{ConfirmTier, Risk};

use crate::guard::policy;
use crate::{ClusterSessionManager, CommandBus};

/// Why a delete was not planned or not run.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DeleteError {
    /// Nothing was selected.
    #[error("nothing is selected to delete")]
    Empty,
    /// The objects belong to more than one cluster; a delete acts on one cluster.
    #[error("the objects to delete belong to more than one cluster")]
    MixedClusters,
    /// The text the user typed is not the one the confirmation asks for. Nothing was sent.
    #[error("type {expected:?} to confirm")]
    NameMismatch {
        /// What the user has to type.
        expected: String,
    },
}

/// One object of a [`DeletePlan`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedDelete {
    /// The object.
    pub target: ResourceRef,
    /// The command that deletes it.
    pub command: Command,
    /// The confirmation the guard will ask for this object.
    pub tier: ConfirmTier,
    /// The command's risk for this object.
    pub risk: Risk,
}

/// What deleting a selection will ask of the user: the objects, the highest confirmation tier
/// among them, and what has to be typed. Built by [`DeleteFlow::plan`] from the same policy the
/// guard applies, so a dialog drawn from it asks what the guard will ask.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeletePlan {
    cluster: ClusterId,
    propagation: Propagation,
    items: Vec<PlannedDelete>,
    tier: ConfirmTier,
    risk: Risk,
    phrase: Option<String>,
}

impl DeletePlan {
    /// The cluster.
    pub fn cluster(&self) -> &ClusterId {
        &self.cluster
    }

    /// How dependents are handled.
    pub fn propagation(&self) -> Propagation {
        self.propagation
    }

    /// The objects, in the order they were selected.
    pub fn items(&self) -> &[PlannedDelete] {
        &self.items
    }

    /// The highest tier among the objects: [`ConfirmTier::Simple`] or
    /// [`ConfirmTier::TypeName`].
    pub fn tier(&self) -> ConfirmTier {
        self.tier
    }

    /// The highest risk among the objects.
    pub fn risk(&self) -> Risk {
        self.risk
    }

    /// For [`ConfirmTier::TypeName`]: the text the user types. The object's name for one object,
    /// the cluster's context name for several (a selection has no single name).
    pub fn phrase(&self) -> Option<&str> {
        self.phrase.as_deref()
    }

    /// How many objects of each kind, by kind name, in alphabetical order: "2 Pods, 1 ConfigMap".
    pub fn kinds(&self) -> Vec<(Arc<str>, usize)> {
        let mut counts: BTreeMap<Arc<str>, usize> = BTreeMap::new();
        for item in &self.items {
            *counts.entry(item.target.gvk.kind.clone()).or_default() += 1;
        }
        counts.into_iter().collect()
    }

    /// The objects that need the typed name (their own tier is [`ConfirmTier::TypeName`]).
    pub fn typed_items(&self) -> impl Iterator<Item = &PlannedDelete> {
        self.items
            .iter()
            .filter(|item| item.tier == ConfirmTier::TypeName)
    }
}

/// Runs deletes through the `CommandBus`, for one object or a selection. See the
/// [module docs](super::super).
///
/// Cheap to clone. It holds no UI: the view builds its dialog from a [`DeletePlan`], calls
/// [`run`](Self::run) on the Tokio bridge (`oxikube_runtime::spawn_kube`) and draws the
/// [`DeleteReport`](super::DeleteReport).
#[derive(Clone)]
pub struct DeleteFlow {
    pub(super) bus: CommandBus,
    sessions: ClusterSessionManager,
    pub(super) who: Arc<str>,
    pub(super) concurrency: usize,
}

impl std::fmt::Debug for DeleteFlow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeleteFlow").finish_non_exhaustive()
    }
}

impl DeleteFlow {
    /// A flow dispatching on `bus` as `who` (the local user's name, for the audit log).
    /// `sessions` supplies the cluster's context name for a multi-object confirmation.
    pub fn new(bus: CommandBus, sessions: ClusterSessionManager, who: impl Into<Arc<str>>) -> Self {
        Self {
            bus,
            sessions,
            who: who.into(),
            concurrency: super::run::CONCURRENCY,
        }
    }

    /// How many objects are deleted at once (at least one). The default is a few; one makes the
    /// order of the requests the order of the selection.
    #[must_use]
    pub fn with_concurrency(mut self, concurrency: usize) -> Self {
        self.concurrency = concurrency.max(1);
        self
    }

    /// Plans deleting `targets` with `propagation`. Sends nothing and asks nobody.
    ///
    /// # Errors
    ///
    /// [`DeleteError::Empty`] for no targets, [`DeleteError::MixedClusters`] for targets of more
    /// than one cluster.
    pub fn plan(
        &self,
        targets: &[ResourceRef],
        propagation: Propagation,
    ) -> Result<DeletePlan, DeleteError> {
        let first = targets.first().ok_or(DeleteError::Empty)?;
        let cluster = first.cluster.clone();
        if targets.iter().any(|t| t.cluster != cluster) {
            return Err(DeleteError::MixedClusters);
        }
        let meta = oxikube_domain::command::lookup(CommandId::RESOURCE_DELETE)
            .expect("resource::Delete is declared");
        let items: Vec<PlannedDelete> = targets
            .iter()
            .map(|target| {
                let command = Command::ResourceDelete {
                    target: target.clone(),
                    propagation,
                };
                PlannedDelete {
                    target: target.clone(),
                    tier: policy::confirm_tier_for(meta, &command),
                    risk: command.effective_risk().or(meta.risk).unwrap_or(Risk::High),
                    command,
                }
            })
            .collect();
        let tier = items
            .iter()
            .map(|item| item.tier)
            .max()
            .unwrap_or(ConfirmTier::Simple);
        let risk = items
            .iter()
            .map(|item| item.risk)
            .max()
            .unwrap_or(Risk::Medium);
        let phrase = (tier == ConfirmTier::TypeName).then(|| match items.as_slice() {
            [only] => only.target.name.to_string(),
            _ => self
                .sessions
                .get(&cluster)
                .map_or_else(|| cluster.to_string(), |s| s.context().to_string()),
        });
        Ok(DeletePlan {
            cluster,
            propagation,
            items,
            tier,
            risk,
            phrase,
        })
    }
}
