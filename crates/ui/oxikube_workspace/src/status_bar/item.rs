//! [`StatusItem`]: what a status-bar indicator implements, and its object-safe handle.

use gpui::{AnyView, App, Entity, EntityId, Render};

/// A status-bar indicator: a small view (read-only badge, connection state, update notice).
///
/// An item is an ordinary entity: it renders itself and calls `cx.notify()` when its state
/// changes. The bar re-renders only then (it observes the item), so an idle status bar costs no
/// frames. Keep `render` cheap: a label, an icon, maybe a click handler.
pub trait StatusItem: Render + 'static {
    /// Whether the item is shown. A hidden item stays registered (and keeps its place) but takes
    /// no room; flip it and `cx.notify()` to show it again. Default: always visible.
    fn visible(&self, _cx: &App) -> bool {
        true
    }
}

/// A [`StatusItem`] entity of any type. Implemented for every `Entity<T: StatusItem>`.
pub trait StatusItemHandle: 'static {
    /// The item's entity id.
    fn item_id(&self) -> EntityId;
    /// The item as a view.
    fn to_any_view(&self) -> AnyView;
    /// See [`StatusItem::visible`].
    fn visible(&self, cx: &App) -> bool;
}

impl<T: StatusItem> StatusItemHandle for Entity<T> {
    fn item_id(&self) -> EntityId {
        self.entity_id()
    }

    fn to_any_view(&self) -> AnyView {
        self.clone().into()
    }

    fn visible(&self, cx: &App) -> bool {
        self.read(cx).visible(cx)
    }
}
