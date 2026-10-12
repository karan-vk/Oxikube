//! The Metrics tab's lifecycle on the view: offered or not, started when shown, one loop that
//! fetches and refreshes, cancelled when the tab or the view goes.

use futures::StreamExt as _;
use gpui::Context;
use oxikube_app::{HistoryAvailability, SubjectKind};
use oxikube_domain::RangePreset;
use oxikube_runtime::spawn_kube;

use super::handle::MetricsHistory;
use super::state::MetricsState;
use crate::detail::tabs::DetailTab;
use crate::detail::view::DetailView;

impl DetailView {
    /// Looks the history service up and follows the cluster's availability, for kinds that have
    /// graphs. Called once from the constructor; reads and spawns nothing else.
    pub(in crate::detail) fn init_metrics(&mut self, cx: &mut Context<Self>) {
        if SubjectKind::of(&self.target.gvk).is_none() {
            return;
        }
        let Some(service) = MetricsHistory::get(cx) else {
            return;
        };
        self.metrics.availability = Some(service.availability(&self.target.cluster));
        let mut changes = service.changes(&self.target.cluster);
        self.metrics.service = Some(service);
        // Waits on the provider service's change stream: no timer, no query. Stops with the view.
        self.metrics.follower = Some(cx.spawn(async move |this, cx| {
            while changes.next().await.is_some() {
                if this
                    .update(cx, |view, cx| view.metrics_availability_changed(cx))
                    .is_err()
                {
                    break;
                }
            }
        }));
    }

    /// Whether the Metrics tab is offered: the kind has graphs and a Prometheus endpoint answered
    /// (or detection is still looking).
    pub fn metrics_offered(&self) -> bool {
        self.metrics.service.is_some()
            && self
                .metrics
                .availability
                .as_ref()
                .is_some_and(HistoryAvailability::offers_tab)
    }

    /// The tabs shown, in order: the kind's, with Metrics only when it is offered.
    pub fn tabs(&self) -> Vec<DetailTab> {
        DetailTab::for_kind(&self.target.gvk)
            .iter()
            .copied()
            .filter(|tab| *tab != DetailTab::Metrics || self.metrics_offered())
            .collect()
    }

    /// How the Metrics tab stands. [`MetricsState::NoEndpoint`] with the reason when it is not
    /// offered.
    pub fn metrics_state(&self) -> MetricsState {
        match &self.metrics.availability {
            Some(HistoryAvailability::Unavailable { reason }) => {
                MetricsState::NoEndpoint(reason.clone())
            }
            _ => self.metrics.state.clone(),
        }
    }

    /// Why the Metrics tab is not there, as the text of a toast: the kind has no graphs, or
    /// `metrics unavailable: <reason>`. `None` when the tab is offered.
    pub fn metrics_unavailable_text(&self) -> Option<String> {
        if SubjectKind::of(&self.target.gvk).is_none() {
            return Some(format!("{} has no history graphs.", self.target.gvk.kind));
        }
        match &self.metrics.availability {
            Some(HistoryAvailability::Unavailable { reason }) => {
                Some(format!("metrics unavailable: {reason}"))
            }
            None => Some("metrics unavailable: this build has no metrics service".to_owned()),
            Some(_) => None,
        }
    }

    /// The range the tab shows.
    pub fn metrics_range(&self) -> RangePreset {
        self.metrics.range
    }

    /// The ids of the charts built, in order.
    pub fn metrics_chart_ids(&self) -> Vec<&'static str> {
        self.metrics.charts.iter().map(|slot| slot.key).collect()
    }

    /// The chart entity `id`, if built.
    pub fn metrics_chart(&self, id: &str) -> Option<gpui::Entity<oxikube_ui::chart::Chart>> {
        self.metrics
            .charts
            .iter()
            .find(|slot| slot.key == id)
            .map(|slot| slot.chart.clone())
    }

    /// How many fetches the tab has started.
    pub fn metrics_fetches(&self) -> u64 {
        self.metrics.fetches
    }

    /// Whether the tab is on screen and should be asking.
    fn metrics_active(&self) -> bool {
        self.tab == DetailTab::Metrics
            && self.shown
            && self.metrics.service.is_some()
            && matches!(
                self.metrics.availability,
                Some(HistoryAvailability::Ready { .. } | HistoryAvailability::Detecting)
            )
    }

    /// The tab was opened: start asking.
    pub(in crate::detail) fn metrics_tab_opened(&mut self, cx: &mut Context<Self>) {
        self.run_metrics(cx);
    }

    /// The tab was left (or the view hidden or released): stop asking. Dropping the loop cancels
    /// the query in flight and the timer; the pods feed of a workload goes with it.
    pub(in crate::detail) fn metrics_tab_closed(&mut self) {
        self.metrics.driver = None;
        self.metrics.looping = false;
        self.metrics.pods = Default::default();
    }

    /// The provider status changed (detection finished, settings edited, cluster dropped).
    fn metrics_availability_changed(&mut self, cx: &mut Context<Self>) {
        let Some(service) = &self.metrics.service else {
            return;
        };
        let now = service.availability(&self.target.cluster);
        if self.metrics.availability.as_ref() == Some(&now) {
            return;
        }
        self.metrics.availability = Some(now);
        if !self.metrics_offered() {
            // The tab disappears; leave it for the Overview and stop asking.
            if self.tab == DetailTab::Metrics {
                self.set_tab(DetailTab::Overview, cx);
            }
            self.metrics_tab_closed();
        } else if self.tab == DetailTab::Metrics {
            self.run_metrics(cx);
        }
        cx.notify();
    }

    /// The object arrived or changed: a workload's selector may be known now.
    pub(in crate::detail) fn metrics_object_changed(&mut self, cx: &mut Context<Self>) {
        if self.metrics_active() && !self.metrics.looping {
            self.run_metrics(cx);
        }
    }

    /// The view was shown or hidden: a hidden tab does not ask.
    pub(in crate::detail) fn metrics_shown_changed(&mut self, cx: &mut Context<Self>) {
        if self.tab != DetailTab::Metrics {
            return;
        }
        if self.shown {
            self.run_metrics(cx);
        } else {
            self.metrics_tab_closed();
        }
    }

    /// Chooses the range (the segmented control). Shows the loading state at once, cancels the
    /// query for the old range and starts one for the new, and remembers the choice.
    pub fn set_metrics_range(&mut self, range: RangePreset, cx: &mut Context<Self>) {
        self.metrics.range_settled = true;
        if self.metrics.range == range {
            return;
        }
        self.metrics.range = range;
        if let Some(service) = self.metrics.service.clone() {
            let saving = spawn_kube(cx, async move { service.save_range(range).await });
            self.metrics.saving = Some(cx.spawn(async move |_, _| {
                saving.await.ok();
            }));
        }
        self.metrics.state = MetricsState::Loading;
        self.run_metrics(cx);
        cx.notify();
    }

    /// Asks again after an error (the Retry button).
    pub fn retry_metrics(&mut self, cx: &mut Context<Self>) {
        if self.metrics.pods.failed.is_some() {
            // A refused pods feed is not retried by the store: subscribe again.
            self.metrics.pods = Default::default();
        }
        self.metrics.state = MetricsState::Loading;
        self.run_metrics(cx);
        cx.notify();
    }

    /// Starts the loop that fetches now and then every `metrics.history_refresh_secs`, replacing
    /// (and so cancelling) a running one. Does nothing while the tab is not active.
    fn run_metrics(&mut self, cx: &mut Context<Self>) {
        self.metrics.driver = None;
        if !self.metrics_active() {
            return;
        }
        let Some(service) = self.metrics.service.clone() else {
            return;
        };
        if SubjectKind::of(&self.target.gvk).is_some_and(SubjectKind::needs_pods) {
            self.follow_metric_pods(cx);
        }
        let settled = self.metrics.range_settled;
        self.metrics.looping = true;
        self.metrics.driver = Some(cx.spawn(async move |this, cx| {
            if !settled {
                let saved = this.update(cx, |_, cx| {
                    let service = service.clone();
                    spawn_kube(cx, async move { service.saved_range().await })
                });
                let Ok(saved) = saved else { return };
                let range = saved.await.unwrap_or(RangePreset::DEFAULT);
                let applied = this.update(cx, |view, cx| {
                    if !view.metrics.range_settled {
                        view.metrics.range = range;
                        view.metrics.range_settled = true;
                        cx.notify();
                    }
                });
                if applied.is_err() {
                    return;
                }
            }
            loop {
                let begun = this.update(cx, |view, cx| view.begin_fetch(&service, cx));
                let work = match begun {
                    Ok(Some(work)) => work,
                    // Nothing to ask yet: the loop ends, and what it waits for (the pods, the
                    // end of detection) starts it again.
                    _ => {
                        this.update(cx, |view, _| view.metrics.looping = false).ok();
                        break;
                    }
                };
                let outcome = work.await;
                let wait = this.update(cx, |view, cx| {
                    match outcome {
                        Ok(results) => view.finish_fetch(results, cx),
                        Err(error) => view.finish_failed(error, cx),
                    }
                    view.refresh_interval(cx)
                });
                let Ok(wait) = wait else { break };
                cx.background_executor().timer(wait).await;
            }
        }));
    }
}
