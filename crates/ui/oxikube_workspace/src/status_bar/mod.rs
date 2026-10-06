//! The status bar: a thin strip under the docks with a left and a right group of indicators.
//!
//! Features register an [`StatusItem`] view with [`StatusBar::register_status_item`] (or
//! `Workspace::register_status_item`) giving the side and a priority. Within a side, items run
//! left to right in ascending priority; equal priorities keep registration order. Typical
//! priorities: 0 for the built-ins (connection state), 100 for feature badges, so a feature can
//! slot in before or after by choosing a number.
//!
//! The bar observes its items, so it re-renders when one of them notifies and never otherwise
//! (docs/PERFORMANCE.md: an idle status bar causes no frames).

mod item;

use gpui::{
    Context, Entity, EntityId, InteractiveElement as _, IntoElement, ParentElement as _, Pixels,
    Render, Styled as _, Subscription, Window, px,
};
use oxikube_ui::{ActiveTokens as _, layout::h_flex, u};

pub use item::{StatusItem, StatusItemHandle};

/// The bar's height in unscaled pixels.
pub const STATUS_BAR_HEIGHT: Pixels = px(24.);

/// Which end of the bar an item sits at.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StatusSide {
    /// The left group (workspace and cluster state).
    Left,
    /// The right group (transient indicators and notices).
    Right,
}

/// Identifies a registered item, for [`StatusBar::remove_status_item`]. It is the item's entity
/// id, so registering the same entity twice replaces its earlier registration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct StatusItemId(pub EntityId);

struct Entry {
    id: StatusItemId,
    side: StatusSide,
    priority: i32,
    handle: Box<dyn StatusItemHandle>,
    _observation: Subscription,
}

/// The status bar view. See the [module docs](self).
#[derive(Default)]
pub struct StatusBar {
    /// Kept sorted by `(priority, registration order)` within each side; the two sides are
    /// interleaved in one vector, which is fine because queries filter by side.
    entries: Vec<Entry>,
}

impl StatusBar {
    /// An empty bar.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds `view` to `side` at `priority` (lower is further left). Registering a view that is
    /// already in the bar moves it to the new side and priority.
    pub fn register_status_item<V: StatusItem>(
        &mut self,
        side: StatusSide,
        priority: i32,
        view: Entity<V>,
        cx: &mut Context<Self>,
    ) -> StatusItemId {
        let id = StatusItemId(view.entity_id());
        self.entries.retain(|entry| entry.id != id);
        let observation = cx.observe(&view, |_, _, cx| cx.notify());
        let entry = Entry {
            id,
            side,
            priority,
            handle: Box::new(view),
            _observation: observation,
        };
        // After every entry of the same side with priority <= ours: stable for ties.
        let at = self
            .entries
            .iter()
            .rposition(|e| e.side == side && e.priority <= priority)
            .map_or_else(
                || self.entries.iter().position(|e| e.side == side),
                |ix| Some(ix + 1),
            )
            .unwrap_or(self.entries.len());
        self.entries.insert(at, entry);
        cx.notify();
        id
    }

    /// Removes an item. Returns whether it was registered.
    pub fn remove_status_item(&mut self, id: StatusItemId, cx: &mut Context<Self>) -> bool {
        let before = self.entries.len();
        self.entries.retain(|entry| entry.id != id);
        let removed = self.entries.len() != before;
        if removed {
            cx.notify();
        }
        removed
    }

    /// The registered items of `side`, in display order (hidden ones included).
    pub fn items(&self, side: StatusSide) -> Vec<StatusItemId> {
        self.entries
            .iter()
            .filter(|entry| entry.side == side)
            .map(|entry| entry.id)
            .collect()
    }

    /// The side and priority an item was registered with.
    pub fn placement(&self, id: StatusItemId) -> Option<(StatusSide, i32)> {
        self.entries
            .iter()
            .find(|entry| entry.id == id)
            .map(|entry| (entry.side, entry.priority))
    }

    fn group(&self, side: StatusSide, id: &'static str, cx: &gpui::App) -> impl IntoElement {
        h_flex()
            .id(id)
            .debug_selector(|| id.to_owned())
            .gap(u(cx.tokens().spacing.md))
            .children(
                self.entries
                    .iter()
                    .filter(|entry| entry.side == side && entry.handle.visible(cx))
                    .map(|entry| entry.handle.to_any_view()),
            )
    }
}

impl Render for StatusBar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let left = self.group(StatusSide::Left, "status-bar-left", cx);
        let right = self.group(StatusSide::Right, "status-bar-right", cx);
        h_flex()
            .id("status-bar")
            .debug_selector(|| "status-bar".to_owned())
            .h(u(STATUS_BAR_HEIGHT))
            .w_full()
            .flex_none()
            .px(u(tokens.spacing.md))
            .justify_between()
            .items_center()
            .bg(colors.surface)
            .border_t_1()
            .border_color(colors.border_variant)
            .text_color(colors.text_muted)
            .text_size(u(tokens.font.small))
            .child(left)
            .child(right)
    }
}
