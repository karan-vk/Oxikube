//! [`PodTally`]: pods by phase and what the scheduled ones ask of the nodes, kept one pod at a
//! time.

use std::collections::HashMap;

use oxikube_domain::view::PodPhase;
use oxikube_domain::{Demand, PodPlacement, Resource};

use crate::store::ObjectKey;

/// How many pods are in each phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PhaseCounts {
    /// `Running`.
    pub running: usize,
    /// `Pending` (scheduled or not).
    pub pending: usize,
    /// `Succeeded`.
    pub succeeded: usize,
    /// `Failed`.
    pub failed: usize,
    /// `Unknown`, missing or a phase this client does not know.
    pub unknown: usize,
}

impl PhaseCounts {
    /// Every pod counted.
    pub fn total(&self) -> usize {
        self.running + self.pending + self.succeeded + self.failed + self.unknown
    }

    /// The count of `phase`.
    pub fn of(&self, phase: PodPhase) -> usize {
        match phase {
            PodPhase::Running => self.running,
            PodPhase::Pending => self.pending,
            PodPhase::Succeeded => self.succeeded,
            PodPhase::Failed => self.failed,
            PodPhase::Unknown => self.unknown,
        }
    }

    fn slot(&mut self, phase: PodPhase) -> &mut usize {
        match phase {
            PodPhase::Running => &mut self.running,
            PodPhase::Pending => &mut self.pending,
            PodPhase::Succeeded => &mut self.succeeded,
            PodPhase::Failed => &mut self.failed,
            PodPhase::Unknown => &mut self.unknown,
        }
    }
}

/// What one pod adds to the tally.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Contribution {
    phase: PodPhase,
    /// What it asks of its node; zero when it is not on one or has finished.
    demand: Demand,
}

impl Contribution {
    fn of(pod: &Resource) -> Self {
        let phase = PodPhase::parse(pod.get_str("/status/phase"));
        let demand = PodPlacement::of_pod(pod).map_or(Demand::ZERO, |p| p.demand);
        Self { phase, demand }
    }
}

/// Pods by phase and the summed [`Demand`] of the pods that occupy a node (scheduled, not
/// finished: `kubectl describe node`'s rule), kept incrementally: [`upsert`](Self::upsert) and
/// [`remove`](Self::remove) cost O(containers) of the pod that changed, never a pass over all
/// pods. The result equals [`PodTally::of`] over the same pods (a property test checks it).
#[derive(Debug, Default, Clone)]
pub struct PodTally {
    by_key: HashMap<ObjectKey, Contribution>,
    phases: PhaseCounts,
    demand: Demand,
}

impl PodTally {
    /// The tally of `pods` in one pass.
    pub fn of<'a>(pods: impl IntoIterator<Item = &'a Resource>) -> Self {
        let mut tally = Self::default();
        for pod in pods {
            tally.upsert(ObjectKey::of(&pod.meta), pod);
        }
        tally
    }

    /// Counts `pod` under `key`, replacing what it counted before. Objects that are not pods are
    /// ignored.
    pub fn upsert(&mut self, key: ObjectKey, pod: &Resource) {
        if !pod.kind.is_pod() {
            return;
        }
        let now = Contribution::of(pod);
        if let Some(before) = self.by_key.insert(key, now) {
            self.subtract(before);
        }
        *self.phases.slot(now.phase) += 1;
        self.demand += now.demand;
    }

    /// Stops counting the pod under `key`.
    pub fn remove(&mut self, key: &ObjectKey) {
        if let Some(before) = self.by_key.remove(key) {
            self.subtract(before);
        }
    }

    /// Forgets every pod.
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    fn subtract(&mut self, before: Contribution) {
        let slot = self.phases.slot(before.phase);
        *slot = slot.saturating_sub(1);
        self.demand -= before.demand;
    }

    /// Pods by phase.
    pub fn phases(&self) -> PhaseCounts {
        self.phases
    }

    /// What the pods on nodes ask for (`pods` is how many occupy a node).
    pub fn demand(&self) -> Demand {
        self.demand
    }

    /// Pods counted.
    pub fn len(&self) -> usize {
        self.by_key.len()
    }

    /// Whether no pod is counted.
    pub fn is_empty(&self) -> bool {
        self.by_key.is_empty()
    }
}
