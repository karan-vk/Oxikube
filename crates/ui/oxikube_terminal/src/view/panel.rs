//! [`TerminalPanel`]: the terminal's side panel in a cluster's bottom dock.
//!
//! Terminal tabs share the bottom dock's tab group with it. The panel is the dock's anchor: the
//! dock area keeps a dock's last tab in place, so with the panel there every terminal tab can be
//! dragged out to a pane, back in, or closed. Its own body is what the dock shows with no terminal
//! displayed: a "New Terminal" button (`terminal::New` for its cluster) and how to dock one.

use std::rc::Rc;

use gpui::{
    Action, App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement as _, Pixels, Render, SharedString,
    Styled as _, Window, div, px,
};
use oxikube_domain::command::Command;
use oxikube_domain::ids::ClusterId;
use oxikube_ui::{ActiveTokens as _, Icon, IconName, button::Button, layout::v_flex, u};
use oxikube_workspace::{CommandDispatcher, DockPosition, Panel, PanelEvent, Workspace};

use super::TogglePanel;

/// What the empty panel says.
const PANEL_HINT: &str = "Terminals of this cluster open here with its kubeconfig, context and namespace set. Drag a \
     terminal tab to a pane, or back here to dock it.";

/// The bottom-dock panel of a cluster's terminals. See the [module docs](self).
pub struct TerminalPanel {
    focus: FocusHandle,
    cluster: Option<ClusterId>,
    dispatcher: Option<Rc<dyn CommandDispatcher>>,
}

impl TerminalPanel {
    /// The panel of `cluster`'s workspace (`None`: a workspace without a cluster), sending
    /// `terminal::New` through `dispatcher`.
    pub fn new(
        cluster: Option<ClusterId>,
        dispatcher: Option<Rc<dyn CommandDispatcher>>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            focus: cx.focus_handle(),
            cluster,
            dispatcher,
        }
    }

    fn new_terminal(&mut self, cx: &mut Context<Self>) {
        if let Some(dispatcher) = &self.dispatcher {
            let cluster = self.cluster.clone();
            dispatcher.dispatch(Command::TerminalNew { cluster }, cx);
        }
    }
}

/// Adds a [`TerminalPanel`] to `workspace`'s bottom dock unless it has one, and returns it. A
/// bottom dock this creates starts closed (the first terminal opens it), so a cluster tab does
/// not lose space to an empty dock; a saved layout restored afterwards sets it as it was.
pub fn ensure_terminal_panel(
    workspace: &Entity<Workspace>,
    cluster: Option<ClusterId>,
    dispatcher: Option<Rc<dyn CommandDispatcher>>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<TerminalPanel> {
    if let Some(panel) = workspace.read(cx).panel::<TerminalPanel>() {
        return panel;
    }
    let panel = cx.new(|cx| TerminalPanel::new(cluster, dispatcher, cx));
    workspace.update(cx, |ws, cx| {
        let had_dock = ws.dock(DockPosition::Bottom, cx).is_some();
        ws.add_panel(panel.clone(), window, cx);
        if !had_dock
            && ws
                .dock(DockPosition::Bottom, cx)
                .is_some_and(|d| d.is_open())
        {
            ws.toggle_dock(DockPosition::Bottom, window, cx);
        }
    });
    panel
}

impl EventEmitter<PanelEvent> for TerminalPanel {}

impl Focusable for TerminalPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for TerminalPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        v_flex()
            .id("terminal-panel")
            .debug_selector(|| "terminal-panel".into())
            .track_focus(&self.focus)
            .size_full()
            .items_center()
            .justify_center()
            .gap(u(tokens.spacing.md))
            .p(u(tokens.spacing.lg))
            .bg(tokens.colors.background)
            .text_color(tokens.colors.text_muted)
            .child(
                Icon::new(IconName::SquareTerminal)
                    .size(u(px(24.)))
                    .color(tokens.colors.text_disabled),
            )
            .child(div().max_w(u(px(460.))).text_center().child(PANEL_HINT))
            .child(
                div().debug_selector(|| "terminal-panel-new".into()).child(
                    Button::new("terminal-panel-new")
                        .label("New Terminal")
                        .on_click(cx.listener(|this, _, _, cx| this.new_terminal(cx))),
                ),
            )
    }
}

impl Panel for TerminalPanel {
    fn persistent_name() -> &'static str {
        "TerminalPanel"
    }

    fn panel_key() -> &'static str {
        "terminal_panel"
    }

    fn position(&self, _: &Window, _: &App) -> DockPosition {
        DockPosition::Bottom
    }

    fn default_size(&self, _: &Window, _: &App) -> Pixels {
        px(280.)
    }

    fn min_size(&self, _: &Window, _: &App) -> Option<Pixels> {
        Some(px(120.))
    }

    fn icon(&self, _: &Window, _: &App) -> Option<IconName> {
        Some(IconName::SquareTerminal)
    }

    fn icon_tooltip(&self, _: &Window, _: &App) -> Option<SharedString> {
        Some("Terminal".into())
    }

    fn title(&self, _: &App) -> SharedString {
        "Terminal".into()
    }

    fn toggle_action(&self) -> Box<dyn Action> {
        Box::new(TogglePanel)
    }

    fn activation_priority(&self) -> u32 {
        10
    }
}
