//! The Nodes [`KindSpec`].

use gpui::AppContext as _;
use oxikube_app::KindMeta;
use oxikube_app::Section;
use oxikube_app::nodes::node_columns;
use oxikube_domain::command::CommandId;
use oxikube_domain::ids::{Gvk, Scope};

use super::panel::NodePanel;
use crate::kinds::{DetailRenderer, KindSpec};

/// A node's own actions: a shell on it (E09-S09), cordon and uncordon (E12-S09). Drain arrives
/// with E12-S10.
const NODE_ACTIONS: [CommandId; 3] = [
    CommandId::NODE_SHELL,
    CommandId::NODE_CORDON,
    CommandId::NODE_UNCORDON,
];

/// The Nodes spec: its sidebar slot, a column provider per cluster, the node panel and the actions.
pub fn spec() -> KindSpec {
    let meta = KindMeta::new(Gvk::new("", "v1", "Node"), "nodes", "Nodes", Scope::Cluster)
        .in_section(Section::NODES, 10)
        .with_columns_per_cluster(|context| node_columns(context, None))
        .with_actions(NODE_ACTIONS);
    KindSpec::new(meta).with_detail(DetailRenderer::panel(|detail, window, cx| {
        cx.new(|cx| NodePanel::new(detail, window, cx)).into()
    }))
}
