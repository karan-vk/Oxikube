//! [`RestorePlan`]: what a saved session turns into against today's catalog.

use std::collections::HashSet;

use oxikube_domain::ids::ClusterId;
use oxikube_ports::ClusterContext;

use super::saved::SavedTabs;

/// A saved cluster that is in no kubeconfig any more.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DroppedCluster {
    /// Its id.
    pub cluster: ClusterId,
    /// The name its tab showed, when it was saved.
    pub title: Option<String>,
}

impl DroppedCluster {
    /// What to call it in a notice: its saved name, else its id.
    pub fn label(&self) -> String {
        self.title
            .clone()
            .unwrap_or_else(|| self.cluster.to_string())
    }
}

/// The clusters to reopen, in tab order, and the ones that are gone.
///
/// Pure: [`resolve`](Self::resolve) reads nothing and starts nothing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RestorePlan {
    /// The catalog entries to reopen, in the saved tab order.
    pub clusters: Vec<ClusterContext>,
    /// The cluster whose tab was displayed, when it is still in the catalog. `None` when the
    /// catalog home was displayed (or the displayed cluster is gone).
    pub active: Option<ClusterId>,
    /// Saved clusters that no kubeconfig defines any more, in saved order.
    pub dropped: Vec<DroppedCluster>,
}

impl RestorePlan {
    /// Matches `saved` against `catalog`. A cluster saved twice is reopened once, at its first
    /// place.
    pub fn resolve(saved: Option<&SavedTabs>, catalog: &[ClusterContext]) -> Self {
        let Some(saved) = saved else {
            return Self::default();
        };
        let mut seen = HashSet::new();
        let mut plan = Self::default();
        for cluster in &saved.open {
            if !seen.insert(cluster) {
                continue;
            }
            match catalog.iter().find(|context| &context.cluster == cluster) {
                Some(context) => plan.clusters.push(context.clone()),
                None => plan.dropped.push(DroppedCluster {
                    cluster: cluster.clone(),
                    title: saved.title(cluster).map(str::to_owned),
                }),
            }
        }
        plan.active = saved
            .active
            .clone()
            .filter(|active| plan.clusters.iter().any(|c| &c.cluster == active));
        plan
    }

    /// The ids to reopen, in tab order.
    pub fn ids(&self) -> impl Iterator<Item = &ClusterId> {
        self.clusters.iter().map(|context| &context.cluster)
    }

    /// Whether there is nothing to reopen.
    pub fn is_empty(&self) -> bool {
        self.clusters.is_empty()
    }

    /// The saved session without the dropped clusters: what is written back so the next launch
    /// does not report them again. `saved` is the row this plan was resolved from.
    pub fn pruned(&self, saved: &SavedTabs) -> SavedTabs {
        let titles = self
            .ids()
            .filter_map(|id| saved.title(id).map(|t| (id.clone(), t.to_owned())));
        SavedTabs::new(self.ids().cloned().collect(), self.active.clone()).with_titles(titles)
    }
}
