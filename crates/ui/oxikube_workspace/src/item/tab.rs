//! [`ItemTab`]: the dock panel that carries one [`Item`](super::Item) in a pane.
//!
//! The dock area (gpui-component's `DockArea`) arranges dock panels; an item is not one, so the
//! workspace wraps each item in an `ItemTab`. The wrapper draws the item's tab label, forwards
//! activation, gates closing on [`Item::can_close`](super::Item::can_close), and reports removal
//! back to the workspace. The item itself never sees the dock.

use gpui::{
    App, Context, CursorStyle, EventEmitter, FocusHandle, Focusable, InteractiveElement as _,
    IntoElement, MouseButton, ParentElement as _, Render, StatefulInteractiveElement as _,
    Styled as _, Window, div, prelude::FluentBuilder as _, px,
};
use oxikube_ui::{
    ActiveTokens as _, Icon, IconName,
    dock::{Panel as DockPanel, PanelBehavior, PanelEvent, PanelInfo, PanelState},
    layout::h_flex,
    u,
};

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
    /// The user clicked the close button of a tab whose item
    /// [intercepts the close](super::Item::intercepts_close): ask the item, do not remove.
    CloseRequested,
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
        // An item that asks before closing gets our own close button (see `title`), because the
        // dock's removes the tab without asking anyone.
        self.item.can_close(cx) && !self.item.intercepts_close(cx)
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
        let own_close = self.item.intercepts_close(cx) && self.item.can_close(cx);
        let title = content.title.clone();
        let label = tab_label(
            format!("tab-{}", content.title),
            content.title,
            content.icon,
            content.dirty,
            content.cluster,
            cx,
        );
        let colors = cx.colors();
        h_flex()
            .gap(u(px(6.)))
            .items_center()
            .child(label)
            .when(own_close, |this| {
                let selector = format!("tab-close-{title}");
                this.child(
                    div()
                        .id(gpui::SharedString::from(selector.clone()))
                        .debug_selector(move || selector)
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(u(px(16.)))
                        .rounded(u(px(3.)))
                        .cursor(CursorStyle::PointingHand)
                        .text_color(colors.text_muted)
                        .hover(|style| style.bg(colors.element_hover))
                        // The tab strip selects and starts drags on mouse down: keep it out.
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(|_, _, _, cx| {
                            cx.stop_propagation();
                            cx.emit(ItemTabEvent::CloseRequested);
                        }))
                        .child(Icon::new(IconName::X).size(u(px(12.)))),
                )
            })
    }

    fn inner_padding(&self, _: &App) -> bool {
        // Items draw edge to edge (tables, editors, terminals).
        false
    }
}
