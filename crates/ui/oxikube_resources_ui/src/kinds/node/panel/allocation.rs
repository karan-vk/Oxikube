//! The panel's allocation section: building the view when something it reads changes, and the
//! usage it polls for (E13-S11).
//!
//! The numbers come from `oxikube_app::allocation` over the E13-S02 sums; this file only decides
//! when to rebuild: when the node, the pods on it, the live usage or the GPU vendor list change,
//! never in `render`.

use gpui::{App, Context};
use oxikube_app::allocate_nodes;
use oxikube_app::allocation::{AllocationView, UsageState};
use oxikube_settings::MetricsSettings;

use super::NodePanel;
use crate::kinds::allocation::{UsageSubject, UsageTarget};

impl NodePanel {
    /// Rebuilds the allocation view from the node, the pods and the usage the panel holds. A node
    /// has about a hundred pods, so this is a pass over a hundred small documents (tens of
    /// microseconds); it runs when one of its inputs changed, not per frame.
    pub(super) fn rebuild_view(&mut self, cx: &App) {
        let (Some(node), Some(pods), Some(detail)) =
            (&self.node, &self.pods, self.detail.upgrade())
        else {
            self.view = None;
            return;
        };
        let cluster = detail.read(cx).target().cluster.clone();
        let gpus = MetricsSettings::gpu_table(cx).for_cluster(&cluster).clone();
        let all = allocate_nodes([&node.resource], pods.resources.iter(), &gpus);
        self.view = all
            .first()
            .map(|alloc| AllocationView::for_node(alloc, &self.usage));
    }

    /// What the usage poll asks, and of whom (`None` while the session has no metrics port yet).
    pub(super) fn usage_target(&self, cx: &App) -> Option<UsageTarget> {
        let detail = self.detail.upgrade()?;
        let node = self.node.as_ref()?.summary.name.clone();
        UsageTarget::of(detail.read(cx), UsageSubject::Node(node))
    }

    /// A new answer from the metrics port.
    pub(super) fn usage_arrived(&mut self, usage: UsageState, cx: &mut Context<Self>) {
        self.usage = usage;
        self.rebuild_view(cx);
        cx.notify();
    }
}
