//! Namespaces: a section that is one kind, with its own module (E12-S16).

use gpui::App;

use super::super::namespace;
use super::super::specs::KindSpecs;

/// Registers Namespaces.
pub fn init(cx: &mut App) {
    if let Err(error) = KindSpecs::register(cx, namespace::spec()) {
        tracing::error!(%error, "the Namespace kind was not registered");
    }
}
