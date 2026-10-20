//! The containers drill-down (E12-S02, k9s' Enter on a pod): a sub-table of the pod's containers
//! at the top of its detail's Overview. Init, sidecar and ephemeral containers are in the same
//! table with a type marker next to the name.
//!
//! [`ContainersPanel`] follows its [`DetailView`]: when the detail's object is a new version (a
//! new JSON document), it rebuilds its [`ContainerRow`]s once, in the observer, and render only
//! lays out the strings it holds. Rows have one height; the table scrolls sideways in a narrow
//! drawer and downwards past `MAX_HEIGHT`.
//!
//! # Usage (E13-S03)
//!
//! Each container's CPU and memory come from the pod's reading in the cluster's samples
//! (`oxikube_app::ClusterUsage`). While the panel is on screen (its detail is shown, on the
//! Overview) it watches the cluster's pods (a `MetricsInterest`), and once a second it compares
//! the samples' epoch with the one its rows were built from: a poll rebuilds the rows once, a
//! still second costs an atomic load. A hidden detail (drawer closed, pinned tab in the
//! background, another tab) drops both, so it keeps no poller alive; showing it again watches
//! again and shows a poll that landed meanwhile at once.

mod forward;
mod render;
mod rows;

#[cfg(test)]
pub(crate) mod tests;

use std::sync::Arc;
use std::time::Duration;

use gpui::{AnyView, App, AppContext as _, Context, Task, WeakEntity, Window};
use oxikube_app::{ClusterUsage, MetricsInterest, MetricsKind, MetricsService};
use oxikube_domain::json::JsonDoc;

pub use rows::{ContainerRow, rows_of};

use crate::detail::DetailView;

/// The panel's tallest height, unscaled pixels: about ten rows; more scroll.
const MAX_HEIGHT: f32 = 300.;

/// How often the panel looks for a new poll.
const USAGE_TICK: Duration = Duration::from_secs(1);

/// The containers sub-table of one pod detail. See the [module docs](self).
pub struct ContainersPanel {
    detail: WeakEntity<DetailView>,
    /// The document the rows were built from.
    doc: Option<JsonDoc>,
    rows: Vec<ContainerRow>,
    /// The pod's TCP container ports, for the Forward buttons (E15-S04).
    ports: Vec<oxikube_portforward_ui::PortChoice>,
    /// How many times the rows were built (tests: once per pod version and poll).
    builds: usize,
    /// The app's metrics service, once set.
    service: Option<MetricsService>,
    /// The pod's cluster's samples, once the app's metrics service is set.
    usage: Option<Arc<ClusterUsage>>,
    /// The samples' epoch the rows were built from.
    epoch: u64,
    /// Keeps the cluster's pods polled while the panel is on screen.
    interest: Option<MetricsInterest>,
    /// Looks for a new poll once a second while the panel is on screen.
    usage_tick: Option<Task<()>>,
}

impl ContainersPanel {
    /// The panel of `detail` (the pod's [`DetailRenderer`](crate::kinds::DetailRenderer)
    /// factory).
    ///
    /// The detail is being drawn when it calls this, so the first rows are built right after
    /// (deferred); until then the panel draws nothing.
    pub fn build(detail: WeakEntity<DetailView>, _: &mut Window, cx: &mut App) -> AnyView {
        let panel = cx.new(|cx| {
            if let Some(view) = detail.upgrade() {
                cx.observe(&view, |this: &mut Self, _, cx| {
                    this.refresh(cx);
                    this.follow_visibility(cx);
                })
                .detach();
            }
            Self {
                detail,
                doc: None,
                rows: Vec::new(),
                ports: Vec::new(),
                builds: 0,
                service: None,
                usage: None,
                epoch: 0,
                interest: None,
                usage_tick: None,
            }
        });
        let first = panel.downgrade();
        cx.defer(move |cx| {
            first
                .update(cx, |panel, cx| {
                    panel.refresh(cx);
                    panel.watch_usage(cx);
                })
                .ok();
        });
        panel.into()
    }

    /// The rows drawn now.
    pub fn rows(&self) -> &[ContainerRow] {
        &self.rows
    }

    /// How many times the rows were built.
    pub fn builds(&self) -> usize {
        self.builds
    }

    /// Whether the panel holds an interest in its cluster's pods now (it is on screen).
    pub fn is_watching(&self) -> bool {
        self.interest.is_some()
    }

    /// Reads the pod's cluster's samples, and watches its pods while the panel is on screen.
    /// Without the app's metrics service, nothing: the usage cells read `–`.
    fn watch_usage(&mut self, cx: &mut Context<Self>) {
        let Some(detail) = self.detail.upgrade() else {
            return;
        };
        let Some(service) = crate::metrics::service(cx) else {
            return;
        };
        let cluster = detail.read(cx).target().cluster.clone();
        self.usage = Some(service.usage(&cluster));
        self.service = Some(service);
        self.follow_visibility(cx);
    }

    /// Holds the interest and the once-a-second check exactly while the detail shows the panel
    /// ([`DetailView::kind_panel_visible`]). Runs on every notify of the detail: a still detail
    /// costs a comparison.
    fn follow_visibility(&mut self, cx: &mut Context<Self>) {
        let (Some(detail), Some(service)) = (self.detail.upgrade(), self.service.as_ref()) else {
            return;
        };
        let visible = detail.read(cx).kind_panel_visible();
        if visible == self.interest.is_some() {
            return;
        }
        if !visible {
            self.interest = None;
            self.usage_tick = None;
            return;
        }
        let cluster = detail.read(cx).target().cluster.clone();
        self.interest = Some(service.watch(&cluster, MetricsKind::Pods));
        self.usage_tick = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(USAGE_TICK).await;
                if this.update(cx, |panel, cx| panel.follow_usage(cx)).is_err() {
                    break;
                }
            }
        }));
        // A poll that landed while the panel was hidden shows now.
        self.follow_usage(cx);
    }

    /// Rebuilds the rows when the detail shows another version of the pod.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.rebuild(false, cx);
    }

    /// Rebuilds the rows when a poll landed since they were built.
    pub(crate) fn follow_usage(&mut self, cx: &mut Context<Self>) {
        let moved = self
            .usage
            .as_ref()
            .is_some_and(|usage| usage.epoch() != self.epoch);
        if moved {
            self.rebuild(true, cx);
        }
    }

    /// Builds the rows from the detail's pod: when it is a new version, or always with `force`.
    fn rebuild(&mut self, force: bool, cx: &mut Context<Self>) {
        let Some(detail) = self.detail.upgrade() else {
            return;
        };
        let usage = self.usage.clone();
        let epoch = usage.as_ref().map_or(0, |usage| usage.epoch());
        let built = detail.read(cx).resource().and_then(|pod| {
            let same = self.doc.as_ref().is_some_and(|doc| doc.ptr_eq(pod.doc()));
            (force || !same).then(|| {
                (
                    pod.doc().clone(),
                    rows_of(pod, usage.as_deref()),
                    oxikube_portforward_ui::ports_of(pod),
                )
            })
        });
        let Some((doc, rows, ports)) = built else {
            return;
        };
        // A poll that changed nothing on screen costs no frame.
        let changed = !force || rows != self.rows || ports != self.ports;
        self.epoch = epoch;
        self.doc = Some(doc);
        self.rows = rows;
        self.ports = ports;
        self.builds += 1;
        if changed {
            cx.notify();
        }
    }
}
