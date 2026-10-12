//! One fetch of the Metrics tab: which charts to ask for, the Tokio-bridge task that asks, and
//! how the answer becomes state and chart entities.

use std::time::Duration;

use gpui::Context;
use oxikube_app::{
    ChartData, ChartDef, HistoryAvailability, HistoryError, HistorySubject, MetricsHistoryService,
    SubjectKind, charts_for,
};
use oxikube_runtime::{KubeTask, KubeTaskError, notify_coalesced, spawn_kube};
use oxikube_settings::MetricsSettings;

use super::charts;
use super::state::{ChartSlot, MetricsState};
use crate::detail::view::DetailView;

/// What one fetch returns: each chart's definition with its result.
pub(super) type Fetched = Vec<(ChartDef, Result<ChartData, HistoryError>)>;

impl DetailView {
    /// The seconds between refreshes, read each time so an edit of the setting applies at once.
    pub(super) fn refresh_interval(&self, cx: &Context<Self>) -> Duration {
        let settings = MetricsSettings::resolve(&self.target.cluster, cx);
        Duration::from_secs(u64::from(settings.history_refresh_secs))
    }

    /// The subject to graph, from the target and (for a workload) the pods followed.
    pub(in crate::detail) fn metrics_subject(&self) -> Option<HistorySubject> {
        let kind = SubjectKind::of(&self.target.gvk)?;
        HistorySubject::new(
            kind,
            self.target.namespace.as_deref(),
            &self.target.name,
            self.metric_pod_names(),
        )
    }

    /// Starts one fetch: the charts of the subject over the range, as a Tokio-bridge task whose
    /// drop cancels the queries. `None` when there is nothing to ask yet (a workload whose pods
    /// have not arrived); the arrival of the pods starts the loop again.
    pub(super) fn begin_fetch(
        &mut self,
        service: &MetricsHistoryService,
        cx: &mut Context<Self>,
    ) -> Option<KubeTask<Fetched>> {
        if !matches!(
            self.metrics.availability,
            Some(HistoryAvailability::Ready { .. })
        ) {
            // Detection is still looking; its result restarts the loop.
            return None;
        }
        if SubjectKind::of(&self.target.gvk).is_some_and(SubjectKind::needs_pods) {
            if let Some(reason) = &self.metrics.pods.failed {
                // No `Ready` will follow a refused feed: say why, and let Retry subscribe again.
                self.metrics.state = MetricsState::failed(format!("cannot list pods: {reason}"));
                cx.notify();
                return None;
            }
            if !self.metrics.pods.ready {
                return None;
            }
        }
        let subject = self.metrics_subject()?;
        self.metrics.pods_left_out = subject.pods_left_out();
        let charts = charts_for(&subject);
        if charts.is_empty() {
            // A workload with no pods: nothing to ask, and the panel says so.
            self.metrics.state = MetricsState::Empty;
            cx.notify();
            return None;
        }
        self.metrics.fetches += 1;
        let (service, cluster, range, end) = (
            service.clone(),
            self.target.cluster.clone(),
            self.metrics.range,
            self.now(),
        );
        Some(spawn_kube(cx, async move {
            let results = service.fetch(&cluster, &charts, range, end).await;
            charts.into_iter().zip(results).collect()
        }))
    }

    /// Takes the results of a fetch: the state, the chart entities, the time.
    pub(super) fn finish_fetch(&mut self, results: Fetched, cx: &mut Context<Self>) {
        self.metrics.state = MetricsState::after_fetch(results.iter().map(|(_, result)| result));
        let range_label = self.metrics.range.label();
        let mut old = std::mem::take(&mut self.metrics.charts);
        let same_charts = old.len() == results.len()
            && old
                .iter()
                .zip(&results)
                .all(|(slot, (def, _))| slot.key == def.id);
        let mut slots = Vec::with_capacity(results.len());
        for (def, result) in &results {
            let slot = match old.iter().position(|slot| slot.key == def.id) {
                Some(at) => {
                    let slot = old.swap_remove(at);
                    // The range may have changed since the chart was made.
                    charts::relabel(&slot.chart, def, range_label, cx);
                    slot
                }
                None => ChartSlot {
                    key: def.id,
                    title: def.title.to_uppercase().into(),
                    chart: charts::new_chart(def, range_label, cx),
                },
            };
            match result {
                Ok(data) => charts::show(&slot.chart, data, self.now_override.is_some(), cx),
                // A chart that failed on its own says why inside its frame.
                Err(error) => {
                    let reason = gpui::SharedString::from(error.text());
                    slot.chart
                        .update(cx, |chart, cx| chart.set_unavailable(Some(reason), cx));
                }
            }
            slots.push(slot);
        }
        self.metrics.charts = slots;
        // A refresh with the same cards leaves the list alone: resetting it would throw away the
        // scroll position and every measured height every few seconds.
        if !same_charts {
            self.metrics.list.reset(self.metrics.charts.len());
        }
        notify_coalesced(cx);
    }

    /// Takes the death of the fetch task (a panic, or the runtime going down): the tab says so
    /// with Retry, instead of showing an answer nobody gave. The loop carries on at its next tick.
    pub(super) fn finish_failed(&mut self, error: KubeTaskError, cx: &mut Context<Self>) {
        self.metrics.state = MetricsState::failed(error);
        notify_coalesced(cx);
    }
}
