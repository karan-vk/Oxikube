//! [`ItemTab`]: the dock panel that carries one [`Item`](super::Item) in a pane.
//!
//! The dock area (gpui-component's `DockArea`) arranges dock panels; an item is not one, so the
//! workspace wraps each item in an `ItemTab`. The wrapper draws the item's tab label, forwards
//! activation, gates closing on [`Item::can_close`](super::Item::can_close), and reports removal
//! back to the workspace. The item itself never sees the dock.

use gpui::{
    App, Context, EventEmitter, FocusHandle, Focusable, IntoElement, ParentElement as _, Render,
    Styled as _, Window, div,
};
use oxikube_ui::dock::{Panel as DockPanel, PanelBehavior, PanelEvent, PanelInfo, PanelState};

use super::ItemHandle;
use crate::tab_label::tab_label;

/// The dock panel name every item tab is persisted under (layout persistence registers its
/// builder under this name). Its [`PanelInfo`] carries the item's
/// `{ "kind", "state" }` (see [`ItemTab::dump`](PanelBehavior::dump)).
pub const ITEM_PANEL_NAME: &str = "oxikube.workspace.Item";

/// What an item tab tells the workspace.
pub(crate) enum ItemTabEvent {
    /// The dock removed the tab for good (closed from its tab bar, or by the workspace).
    Removed,
}

/// A dock panel showing one item.
pub(crate) struct ItemTab {
    item: Box<dyn ItemHandle>,
}

impl ItemTab {
    pub(crate) fn new(item: Box<dyn ItemHandle>) -> Self {
        Self { item }
    }
}

impl EventEmitter<ItemTabEvent> for ItemTab {}
impl EventEmitter<PanelEvent> for ItemTab {}

impl Focusable for ItemTab {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        // The dock focuses the displayed panel when its tab is selected: hand it the item's
        // handle so the item, not the wrapper, receives keystrokes.
        self.item.focus_handle(cx)
    }
}

impl Render for ItemTab {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.item.to_any_view())
    }
}

impl PanelBehavior for ItemTab {
    fn panel_name(&self) -> &'static str {
        ITEM_PANEL_NAME
    }

    fn closable(&self, cx: &App) -> bool {
        self.item.can_close(cx)
    }

    fn set_active(&mut self, active: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.item.set_active(active, window, cx);
    }

    fn on_removed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.item.on_close(window, cx);
        cx.emit(ItemTabEvent::Removed);
    }

    fn dump(&self, cx: &App) -> PanelState {
        let mut state = PanelState::new(ITEM_PANEL_NAME);
        if let (Some(kind), Some(item_state)) =
            (self.item.serialized_kind(), self.item.serialize(cx))
        {
            state.info = PanelInfo::panel(serde_json::json!({
                "kind": kind,
                "state": item_state,
            }));
        }
        state
    }
}

impl DockPanel for ItemTab {
    fn title(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = self.item.tab_content(cx);
        tab_label(
            format!("tab-{}", content.title),
            content.title,
            content.icon,
            content.dirty,
            cx,
        )
    }

    fn inner_padding(&self, _: &App) -> bool {
        // Items draw edge to edge (tables, editors, terminals).
        false
    }
}
