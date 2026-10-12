//! [`DrainBoard`]: a running drain as rows, one per pod, folded from its progress events.

use std::collections::HashMap;
use std::time::Duration;

use super::options::{DrainProgress, DrainSummary, PodRef, SkippedPod};

/// Where one pod of a drain is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PodDrainState {
    /// Planned; no eviction sent yet.
    Pending,
    /// An eviction request is in flight.
    Evicting {
        /// 1 for the first request, one more per retry.
        attempt: u32,
    },
    /// A PodDisruptionBudget (or the server's rate limiting) refused the eviction; it is retried.
    BlockedByPdb {
        /// The attempt that was refused.
        attempt: u32,
        /// The server's explanation, one redacted line.
        reason: String,
        /// How long until the next attempt.
        retry_in: Duration,
    },
    /// The eviction was accepted; the pod is terminating.
    Terminating,
    /// The pod is gone from the node.
    Deleted,
    /// The pod could not be evicted, or did not go in time.
    Failed {
        /// What went wrong, one redacted line.
        error: String,
    },
}

impl PodDrainState {
    /// One word for a status column.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Pending => "Pending",
            Self::Evicting { .. } => "Evicting",
            Self::BlockedByPdb { .. } => "Blocked by PDB",
            Self::Terminating => "Terminating",
            Self::Deleted => "Deleted",
            Self::Failed { .. } => "Failed",
        }
    }
}

/// One pod of a drain and where it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrainRow {
    /// The pod.
    pub pod: PodRef,
    /// Where it is.
    pub state: PodDrainState,
}

/// Where the drain as a whole is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DrainPhase {
    /// Asked for; reading the node and its pods.
    Starting,
    /// Planned; the node is being cordoned.
    Planned,
    /// Cordoned; pods are being evicted.
    Evicting,
    /// Every pod is settled; see the summary.
    Finished(DrainSummary),
    /// The drain stopped as a whole (refused, forbidden, the node is gone).
    Failed(String),
    /// Stopped by the user. The node stays cordoned; evictions already sent stay sent.
    Cancelled,
}

/// The counts a progress line shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DrainCounts {
    /// Pods the drain evicts.
    pub total: usize,
    /// Pods gone.
    pub deleted: usize,
    /// Pods that failed.
    pub failed: usize,
    /// Pods a budget is holding back right now.
    pub blocked: usize,
}

/// A drain as the dialog shows it: the phase, one row per pod in plan order, and the pods left
/// alone. Pure: [`apply`](Self::apply) folds a [`DrainProgress`] in, nothing else changes it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DrainBoard {
    phase: Option<DrainPhase>,
    rows: Vec<DrainRow>,
    skipped: Vec<SkippedPod>,
    /// `(namespace, name)` to the row index.
    index: HashMap<(String, String), usize>,
    /// Whether the node was cordoned before this drain (`None` until the cordon).
    was_cordoned: Option<bool>,
}

impl DrainBoard {
    /// An empty board in [`DrainPhase::Starting`].
    pub fn new() -> Self {
        Self::default()
    }

    /// Where the drain is.
    pub fn phase(&self) -> &DrainPhase {
        self.phase.as_ref().unwrap_or(&DrainPhase::Starting)
    }

    /// The pods to evict, in plan order (a pod the second look after the cordon found is last).
    pub fn rows(&self) -> &[DrainRow] {
        &self.rows
    }

    /// The pods left alone.
    pub fn skipped(&self) -> &[SkippedPod] {
        &self.skipped
    }

    /// Whether the node was unschedulable before the drain; `None` until it is cordoned.
    pub fn was_cordoned(&self) -> Option<bool> {
        self.was_cordoned
    }

    /// Whether the drain is still going.
    pub fn is_running(&self) -> bool {
        matches!(
            self.phase(),
            DrainPhase::Starting | DrainPhase::Planned | DrainPhase::Evicting
        )
    }

    /// The counts of the rows.
    pub fn counts(&self) -> DrainCounts {
        let mut counts = DrainCounts {
            total: self.rows.len(),
            ..DrainCounts::default()
        };
        for row in &self.rows {
            match row.state {
                PodDrainState::Deleted => counts.deleted += 1,
                PodDrainState::Failed { .. } => counts.failed += 1,
                PodDrainState::BlockedByPdb { .. } => counts.blocked += 1,
                _ => {}
            }
        }
        counts
    }

    /// Folds one step in.
    pub fn apply(&mut self, step: &DrainProgress) {
        match step {
            DrainProgress::Planned { evict, skipped } => {
                for pod in evict {
                    self.row_mut(pod);
                }
                self.skipped.clone_from(skipped);
                self.phase = Some(DrainPhase::Planned);
            }
            DrainProgress::Cordoned { already } => {
                self.was_cordoned = Some(*already);
                self.phase = Some(DrainPhase::Evicting);
            }
            DrainProgress::Evicting { pod, attempt } => {
                self.set(pod, PodDrainState::Evicting { attempt: *attempt });
            }
            DrainProgress::Blocked {
                pod,
                attempt,
                reason,
                retry_in,
            } => self.set(
                pod,
                PodDrainState::BlockedByPdb {
                    attempt: *attempt,
                    reason: reason.clone(),
                    retry_in: *retry_in,
                },
            ),
            DrainProgress::Evicted { pod } => self.set(pod, PodDrainState::Terminating),
            DrainProgress::Gone { pod } => self.set(pod, PodDrainState::Deleted),
            DrainProgress::PodFailed { pod, error } => self.set(
                pod,
                PodDrainState::Failed {
                    error: error.clone(),
                },
            ),
            DrainProgress::Finished(summary) => {
                // The summary is the truth: rows it names as failed are failed even when no
                // event said so, and a dry run's rows stay as planned.
                if !summary.dry_run {
                    self.drop_vanished(summary);
                }
                for pod in &summary.failed {
                    if !matches!(self.row_mut(pod).state, PodDrainState::Failed { .. }) {
                        self.set(
                            pod,
                            PodDrainState::Failed {
                                error: "not evicted".to_owned(),
                            },
                        );
                    }
                }
                self.skipped.clone_from(&summary.skipped);
                self.phase = Some(DrainPhase::Finished(summary.clone()));
            }
        }
    }

    /// The drain failed as a whole with `error` (one line). Rows keep their state.
    pub fn fail(&mut self, error: impl Into<String>) {
        if self.is_running() {
            self.phase = Some(DrainPhase::Failed(error.into()));
        }
    }

    /// The user stopped the drain. Rows keep their state.
    pub fn cancel(&mut self) {
        if self.is_running() {
            self.phase = Some(DrainPhase::Cancelled);
        }
    }

    /// Drops the rows still `Pending` that `summary` names neither as evicted nor as failed:
    /// pods the first plan listed that were gone (finished or deleted) by the second look
    /// after the cordon, so nothing was ever sent for them and no event will settle them.
    fn drop_vanished(&mut self, summary: &DrainSummary) {
        let named = |pod: &PodRef| {
            summary
                .evicted
                .iter()
                .chain(&summary.failed)
                .any(|other| other.namespace == pod.namespace && other.name == pod.name)
        };
        let before = self.rows.len();
        self.rows
            .retain(|row| row.state != PodDrainState::Pending || named(&row.pod));
        if self.rows.len() != before {
            self.index = self
                .rows
                .iter()
                .enumerate()
                .map(|(at, row)| ((row.pod.namespace.clone(), row.pod.name.clone()), at))
                .collect();
        }
    }

    fn set(&mut self, pod: &PodRef, state: PodDrainState) {
        self.row_mut(pod).state = state;
    }

    /// The row of `pod`, added (as `Pending`) when the plan did not name it.
    fn row_mut(&mut self, pod: &PodRef) -> &mut DrainRow {
        let key = (pod.namespace.clone(), pod.name.clone());
        let next = self.rows.len();
        let at = *self.index.entry(key).or_insert(next);
        if at == next {
            self.rows.push(DrainRow {
                pod: pod.clone(),
                state: PodDrainState::Pending,
            });
        }
        &mut self.rows[at]
    }
}
