//! Namespaces: a section that is one kind.

use gpui::App;
use oxikube_app::Section;

use super::{Builtin, cluster, generic, register};

const NAMESPACES: &[Builtin] = &[cluster("", "v1", "Namespace", "namespaces", "Namespaces")];

/// Registers Namespaces.
pub fn init(cx: &mut App) {
    register(cx, Section::NAMESPACES, NAMESPACES, generic);
}
