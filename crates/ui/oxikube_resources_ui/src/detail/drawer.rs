//! [`DetailDrawer`]: the right-hand dock of a cluster tab, hosting one [`DetailView`].
//!
//! The drawer is a workspace [`Panel`]; the detail it shows is the same entity that can be a
//! tab. Pinning ([`DetailDrawer::take`]) hands that entity to the workspace as an [`Item`]
//! (`oxikube_workspace`), so the active tab, the scroll and the expanded values carry over, and
//! the tab then moves between panes like any other.
//!
//! [`Item`]: oxikube_workspace::Item

use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
    ParentElement as _, Pixels, Render, SharedString, Styled as _, Subscription, Window, actions,
    div, px,
};
use oxikube_domain::ids::ResourceRef;
use oxikube_ui::layout::v_flex;
use oxikube_ui::{ActiveTokens as _, IconName, u};
use oxikube_workspace::{DockPosition, Panel, PanelEvent};

use super::state::{DetailDeps, DetailEvent, Mount};
use super::view::DetailView;

actions!(
    resource_detail,
    [
        /// Opens or closes the detail drawer (the panel's toggle action; no key yet).
        ToggleDrawer,
    ]
);

/// The default width of the drawer's dock, unscaled pixels.
pub const DEFAULT_WIDTH: f32 = 460.;

/// The detail drawer of one cluster tab. See the module docs.
pub struct DetailDrawer {
    focus: FocusHandle,
    view: Option<Entity<DetailView>>,
    /// Hears the view's close button; replaced with the view.
    close: Option<Subscription>,
}

impl EventEmitter<PanelEvent> for DetailDrawer {}

impl DetailDrawer {
    /// An empty drawer.
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            view: None,
            close: None,
        }
    }

    /// The detail shown, if any.
    pub fn view(&self) -> Option<&Entity<DetailView>> {
        self.view.as_ref()
    }

    /// The target of the detail shown.
    pub fn target<'a>(&'a self, cx: &'a App) -> Option<&'a ResourceRef> {
        self.view.as_ref().map(|view| view.read(cx).target())
    }

    /// Shows the detail of `target` and opens the drawer. The detail already shown for the same
    /// object stays (and keeps its tab and scroll); another object replaces it.
    pub fn show(
        &mut self,
        target: ResourceRef,
        deps: &DetailDeps,
        cx: &mut Context<Self>,
    ) -> Entity<DetailView> {
        if let Some(view) = &self.view
            && view.read(cx).target() == &target
        {
            let view = view.clone();
            cx.emit(PanelEvent::Activate);
            return view;
        }
        let deps = deps.clone();
        let view = cx.new(|cx| DetailView::new(target, deps, Mount::Drawer, cx));
        self.close = Some(
            cx.subscribe(&view, |this, _, event: &DetailEvent, cx| match event {
                DetailEvent::Close => this.close(cx),
            }),
        );
        self.view = Some(view.clone());
        cx.emit(PanelEvent::Activate);
        cx.notify();
        view
    }

    /// Closes the drawer and drops its detail (the feeds are released).
    pub fn close(&mut self, cx: &mut Context<Self>) {
        if let Some(view) = self.view.take() {
            view.update(cx, |view, _| view.release());
        }
        self.close = None;
        cx.emit(PanelEvent::Close);
        cx.notify();
    }

    /// Takes the detail of `target` out of the drawer, to be pinned as a tab: the entity itself
    /// (its state intact), mounted as a tab, and the drawer closes. `None` when the drawer shows
    /// something else.
    pub fn take(
        &mut self,
        target: &ResourceRef,
        cx: &mut Context<Self>,
    ) -> Option<Entity<DetailView>> {
        if self.view.as_ref()?.read(cx).target() != target {
            return None;
        }
        let view = self.view.take()?;
        self.close = None;
        view.update(cx, |view, cx| view.set_mount(Mount::Tab, cx));
        cx.emit(PanelEvent::Close);
        cx.notify();
        Some(view)
    }
}

impl Focusable for DetailDrawer {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        match &self.view {
            Some(view) => view.read(cx).focus_handle(cx),
            None => self.focus.clone(),
        }
    }
}

impl Render for DetailDrawer {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        match &self.view {
            Some(view) => v_flex().size_full().child(view.clone()).into_any_element(),
            None => div()
                .size_full()
                .p(u(tokens.spacing.xl))
                .text_color(tokens.colors.text_muted)
                .child("Select an object to see its detail.")
                .into_any_element(),
        }
    }
}

impl Panel for DetailDrawer {
    fn persistent_name() -> &'static str {
        "ResourceDetailDrawer"
    }

    fn panel_key() -> &'static str {
        "resource_detail_drawer"
    }

    fn position(&self, _: &Window, _: &App) -> DockPosition {
        DockPosition::Right
    }

    fn default_size(&self, _: &Window, _: &App) -> Pixels {
        px(DEFAULT_WIDTH)
    }

    fn min_size(&self, _: &Window, _: &App) -> Option<Pixels> {
        Some(px(320.))
    }

    fn icon(&self, _: &Window, _: &App) -> Option<IconName> {
        Some(IconName::PanelRight)
    }

    fn icon_tooltip(&self, _: &Window, _: &App) -> Option<SharedString> {
        Some("Detail".into())
    }

    fn title(&self, _: &App) -> SharedString {
        "Detail".into()
    }

    fn toggle_action(&self) -> Box<dyn gpui::Action> {
        Box::new(ToggleDrawer)
    }

    fn activation_priority(&self) -> u32 {
        10
    }

    fn set_active(&mut self, active: bool, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(view) = &self.view {
            view.update(cx, |view, cx| view.set_shown(active, cx));
        }
    }
}
