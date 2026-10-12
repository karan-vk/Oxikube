//! The Nodes [`KindSpec`].

use gpui::AppContext as _;
use oxikube_app::KindMeta;
use oxikube_app::Section;
use oxikube_app::nodes::node_columns;
use oxikube_domain::command::CommandId;
use oxikube_domain::ids::{Gvk, Scope};

use super::panel::NodePanel;
use crate::kinds::{DetailRenderer, KindSpec};
use crate::metrics::MetricsSlot;

/// A node's own actions: a shell on it (E09-S09), cordon and uncordon (E12-S09), drain
/// (E12-S10).
const NODE_ACTIONS: [CommandId; 4] = [
    CommandId::NODE_SHELL,
    CommandId::NODE_CORDON,
    CommandId::NODE_UNCORDON,
    CommandId::NODE_DRAIN,
];

/// The Nodes spec: its sidebar slot, a column provider per cluster (CPU% and Mem% from the
/// cluster's samples in `metrics`, E13-S03), the node panel and the actions.
pub fn spec(metrics: MetricsSlot) -> KindSpec {
    let meta = KindMeta::new(Gvk::new("", "v1", "Node"), "nodes", "Nodes", Scope::Cluster)
        .in_section(Section::NODES, 10)
        .with_columns_per_cluster(move |context| {
            node_columns(context, metrics.node_source(&context.cluster))
        })
        .with_actions(NODE_ACTIONS);
    KindSpec::new(meta).with_detail(DetailRenderer::panel(|detail, window, cx| {
        cx.new(|cx| NodePanel::new(detail, window, cx)).into()
    }))
}
