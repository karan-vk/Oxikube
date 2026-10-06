//! [`PanelTab`]: the dock panel that carries one [`Panel`](super::Panel) in a dock.

use gpui::{
    App, Context, EventEmitter, FocusHandle, Focusable, IntoElement, ParentElement as _, Render,
    Styled as _, Window, div,
};
use oxikube_ui::dock::{
    Panel as DockPanel, PanelBehavior, PanelEvent as DockPanelEvent, PanelInfo, PanelState,
};

use super::PanelHandle;
use crate::tab_label::tab_label;

/// A dock panel showing one side panel. Side panels are not closed from their tab: their dock is
/// toggled instead, so the wrapper reports itself not closable.
pub(crate) struct PanelTab {
    panel: Box<dyn PanelHandle>,
}

impl PanelTab {
    pub(crate) fn new(panel: Box<dyn PanelHandle>) -> Self {
        Self { panel }
    }
}

impl EventEmitter<DockPanelEvent> for PanelTab {}

impl Focusable for PanelTab {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.panel.focus_handle(cx)
    }
}

impl Render for PanelTab {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.panel.to_any_view())
    }
}

impl PanelBehavior for PanelTab {
    fn panel_name(&self) -> &'static str {
        self.panel.persistent_name()
    }

    fn closable(&self, _: &App) -> bool {
        false
    }

    fn set_active(&mut self, active: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.panel.set_active(active, window, cx);
    }

    fn set_zoomed(&mut self, zoomed: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.panel.set_zoomed(zoomed, window, cx);
    }

    fn dump(&self, cx: &App) -> PanelState {
        let mut state = PanelState::new(self.panel.persistent_name());
        if let Some(panel_state) = self.panel.serialize(cx) {
            state.info = PanelInfo::panel(serde_json::json!({
                "key": self.panel.panel_key(),
                "state": panel_state,
            }));
        }
        state
    }
}

impl DockPanel for PanelTab {
    fn title(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let title = self.panel.title(cx);
        tab_label(
            format!("panel-tab-{title}"),
            title,
            self.panel.icon(window, cx),
            false,
            None,
            cx,
        )
    }

    fn inner_padding(&self, _: &App) -> bool {
        false
    }
}
