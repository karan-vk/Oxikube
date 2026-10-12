//! The cluster CPU and memory gauges: metrics-server's node readings against the nodes'
//! allocatable.
//!
//! The figures come from [`cluster_utilisation`] over the per-node rings the metrics service
//! keeps, so a page that opens late has its sparkline at once. The sum runs on the background
//! executor and is redone only when the service published a poll (its epoch moved) or the node
//! count changed, never on a timer. When there are no metrics the tiles say so with the service's
//! words (`metrics unavailable: <reason>`), never a zero.

use gpui::Context;
use oxikube_app::{ClusterUtilisation, CountState, MetricsKind, MetricsState, cluster_utilisation};
use oxikube_domain::Resource;
use oxikube_domain::ids::Gvk;
use oxikube_domain::utilisation::GpuVendors;
use oxikube_runtime::notify_coalesced;

use super::face::ClusterFigure;
use super::specs::{CLUSTER_CPU, CLUSTER_MEMORY};
use super::view::PulsesView;

impl PulsesView {
    /// Brings the gauges up to date with the metrics service: its state, its thresholds and, when
    /// samples are there and changed, the sums. Cheap when nothing changed.
    pub(super) fn refresh_cluster(&mut self, cx: &mut Context<Self>) {
        if !self.active || self.store.is_none() {
            return;
        }
        let Some(metrics) = self.deps.metrics.clone() else {
            self.show_gauges(
                ClusterFigure::Unavailable("metrics unavailable: not started".to_owned()),
                cx,
            );
            return;
        };
        let usage = metrics.usage(&self.cluster);
        if self.model.set_thresholds(usage.thresholds()) {
            notify_coalesced(cx);
        }
        match usage.state(MetricsKind::Nodes) {
            MetricsState::Available { .. } => {}
            MetricsState::Idle | MetricsState::Loading => {
                if !self
                    .model
                    .tile(CLUSTER_CPU)
                    .is_some_and(|t| matches!(t.figure, ClusterFigure::Reading { .. }))
                {
                    self.show_gauges(ClusterFigure::Waiting, cx);
                }
                return;
            }
            unavailable => {
                let text = unavailable
                    .unavailable_text()
                    .unwrap_or_else(|| "metrics unavailable".to_owned());
                self.gauge_stamp = None;
                self.show_gauges(ClusterFigure::Unavailable(text), cx);
                return;
            }
        }
        // The sums need the nodes' allocatable, so the nodes must be listed. When they cannot be,
        // the gauges say why rather than wait for ever.
        let nodes = match self.model.tile("nodes").map(|t| &t.count) {
            Some(CountState::Counted(count)) => count.total,
            Some(CountState::NoAccess { .. }) => {
                return self.nodes_missing("you may not list the nodes", cx);
            }
            Some(CountState::OverBudget { .. }) => {
                return self.nodes_missing("the nodes are over the watch budget", cx);
            }
            Some(CountState::Failed { message }) => {
                let reason = format!("the nodes could not be listed: {message}");
                return self.nodes_missing(&reason, cx);
            }
            Some(CountState::NotWatched) if self.settled() => {
                return self.nodes_missing("the nodes are not watched", cx);
            }
            // Still listing (the tick looks again).
            _ => return,
        };
        let stamp = (usage.epoch(), nodes);
        if self.gauge_stamp == Some(stamp) {
            return;
        }
        let Some(objects) = self
            .store
            .as_ref()
            .and_then(|store| store.cluster_objects(&Gvk::new("", "v1", "Node")))
        else {
            return;
        };
        self.gauge_stamp = Some(stamp);
        self.gauge_task = Some(cx.spawn(async move |this, cx| {
            let sums = cx
                .background_executor()
                .spawn(async move {
                    let resources: Vec<&Resource> =
                        objects.iter().filter_map(|o| o.resource()).collect();
                    cluster_utilisation(&usage, resources, &GpuVendors::none())
                })
                .await;
            this.update(cx, |this, cx| this.apply_utilisation(sums, cx))
                .ok();
        }));
    }

    /// Metrics are there but the nodes they are measured against are not: both gauges say so.
    fn nodes_missing(&mut self, reason: &str, cx: &mut Context<Self>) {
        self.gauge_stamp = None;
        self.gauge_task = None;
        self.show_gauges(
            ClusterFigure::Unavailable(format!("metrics unavailable: {reason}")),
            cx,
        );
    }

    fn apply_utilisation(&mut self, sums: Option<ClusterUtilisation>, cx: &mut Context<Self>) {
        let Some(sums) = sums else {
            self.show_gauges(
                ClusterFigure::Unavailable(
                    "metrics unavailable: no node reports its usage".to_owned(),
                ),
                cx,
            );
            return;
        };
        let mut changed = false;
        for (id, use_) in [(CLUSTER_CPU, &sums.cpu), (CLUSTER_MEMORY, &sums.memory)] {
            let history = use_
                .history
                .iter()
                .map(|(at, percent)| (at.as_second() as f64, *percent))
                .collect();
            changed |= self.model.set_cluster(
                id,
                ClusterFigure::Reading {
                    used: use_.used,
                    allocatable: use_.allocatable,
                    percent: use_.percent,
                    nodes: sums.nodes,
                    nodes_reporting: sums.nodes_reporting,
                },
                history,
            );
        }
        if changed {
            notify_coalesced(cx);
        }
    }

    /// Both gauges show `figure` and no history.
    fn show_gauges(&mut self, figure: ClusterFigure, cx: &mut Context<Self>) {
        let mut changed = false;
        for id in [CLUSTER_CPU, CLUSTER_MEMORY] {
            changed |= self.model.set_cluster(id, figure.clone(), Vec::new());
        }
        if changed {
            notify_coalesced(cx);
        }
    }
}
