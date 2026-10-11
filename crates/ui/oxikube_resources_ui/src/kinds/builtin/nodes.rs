//! Nodes: a section that is one kind.

use gpui::App;
use oxikube_app::Section;
use oxikube_domain::command::CommandId;

use super::{Builtin, cluster, register};

const NODES: &[Builtin] = &[cluster("", "v1", "Node", "nodes", "Nodes")];

/// A node's own actions: a shell on it (E09-S09). Cordon, uncordon and drain arrive with E12-S09
/// and E12-S10.
const NODE_ACTIONS: &[CommandId] = &[CommandId::NODE_SHELL];

/// Registers Nodes.
pub fn init(cx: &mut App) {
    register(cx, Section::NODES, NODES, |_| NODE_ACTIONS);
}
