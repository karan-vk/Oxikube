//! [`MetricsSettingsView`]: the GPUI entity of the cluster's metrics page.
//!
//! | File | Holds |
//! |---|---|
//! | `mod.rs` | the struct, the change subscription, the buttons' commands |
//! | `render.rs` | the frame: header, headline, services looked at, settings in effect |
//! | `item.rs` | the workspace `Item` |

mod item;
mod render;

pub use item::ITEM_KEY;

use std::rc::Rc;

use futures::StreamExt as _;
use gpui::{Context, EventEmitter, FocusHandle, Focusable, ScrollHandle, Task};
use oxikube_domain::command::Command;
use oxikube_domain::ids::ClusterId;
use oxikube_workspace::{CommandDispatcher, ItemEvent};

use super::backend::MetricsBackend;
use super::model::MetricsSettingsModel;

/// What the page needs from the outside. All of it is handed in, so a test builds the view over
/// fakes and the app builds it over its service.
#[derive(Clone)]
pub struct MetricsDeps {
    /// Where the status comes from.
    pub backend: Rc<dyn MetricsBackend>,
    /// Where the buttons' commands go (the bus).
    pub dispatcher: Rc<dyn CommandDispatcher>,
}

/// The metrics page of one cluster. See the [module docs](super).
pub struct MetricsSettingsView {
    cluster: ClusterId,
    deps: MetricsDeps,
    model: MetricsSettingsModel,
    focus: FocusHandle,
    scroll: ScrollHandle,
    /// Re-reads the status when the service says it changed. Lives as long as the view.
    _watch: Task<()>,
}

impl EventEmitter<ItemEvent> for MetricsSettingsView {}

impl Focusable for MetricsSettingsView {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl MetricsSettingsView {
    /// The page of `cluster`, showing the status the backend has now and following its changes.
    pub fn new(cluster: ClusterId, deps: MetricsDeps, cx: &mut Context<Self>) -> Self {
        let model = build_model(&cluster, &deps);
        let mut changes = deps.backend.changes(&cluster);
        let watch = cx.spawn(async move |this, cx| {
            while changes.next().await.is_some() {
                if this.update(cx, |this, cx| this.refresh(cx)).is_err() {
                    break;
                }
            }
        });
        Self {
            cluster,
            deps,
            model,
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            _watch: watch,
        }
    }

    /// The cluster the page is about.
    pub fn cluster(&self) -> &ClusterId {
        &self.cluster
    }

    /// What the page shows now.
    pub fn model(&self) -> &MetricsSettingsModel {
        &self.model
    }

    /// Reads the status again and redraws when it differs from what is shown.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let model = build_model(&self.cluster, &self.deps);
        if model != self.model {
            self.model = model;
            cx.notify();
        }
    }

    /// Runs detection again: `cluster::RedetectMetrics` on the bus.
    pub fn redetect(&mut self, cx: &mut Context<Self>) {
        let command = Command::ClusterRedetectMetrics {
            cluster: self.cluster.clone(),
        };
        self.deps.dispatcher.dispatch(command, cx);
    }

    /// Switches the cluster's history metrics on or off: `cluster::ToggleMetrics` on the bus.
    pub fn toggle_enabled(&mut self, cx: &mut Context<Self>) {
        let command = Command::ClusterToggleMetrics {
            cluster: self.cluster.clone(),
            enabled: Some(!self.model.enabled),
        };
        self.deps.dispatcher.dispatch(command, cx);
    }
}

fn build_model(cluster: &ClusterId, deps: &MetricsDeps) -> MetricsSettingsModel {
    let source = deps.backend.page(cluster);
    MetricsSettingsModel::build(cluster, &source.title, source.connected, &source.status)
}
