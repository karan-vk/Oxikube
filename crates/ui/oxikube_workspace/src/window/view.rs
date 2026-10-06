//! The content of the main window: the title bar over the [`Workspace`].
//!
//! This is the view the `Root` hosts. The workspace fills the body: docks and centre panes, the
//! status bar below them, and the toast and modal layers over them (E05-S10).
//!
//! # Startup placeholder (E05-S13)
//!
//! The window never waits for the disk: [`MainView::restoring`] shows the workspace at once in its
//! default (empty) layout and reads the saved layout in the background
//! ([`LayoutPersistence`]). That default layout is the placeholder: it is fully interactive (key
//! bindings, actions, opening items) and the title bar says "Restoring layout…" until the read
//! finishes. Then the saved layout replaces it; when the read fails, or nothing was saved, the
//! placeholder simply stays as the usable default layout and the marker goes away.

use gpui::{
    App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, Styled as _, Subscription, Window, div,
    prelude::FluentBuilder as _, px,
};
use oxikube_ui::{ActiveTokens as _, layout::v_flex, title_bar::TitleBar};

use super::options::WINDOW_TITLE;
use crate::persistence::{LayoutPersistence, LayoutStore};
use crate::workspace::Workspace;

/// What the title bar shows while the saved layout is being read.
pub const RESTORING_LABEL: &str = "Restoring layout…";

/// Root content view of the main window.
pub struct MainView {
    workspace: Entity<Workspace>,
    persistence: Option<Entity<LayoutPersistence>>,
    _observe_restore: Option<Subscription>,
}

impl MainView {
    /// The title bar over a new, empty workspace that is not persisted.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            workspace: cx.new(|cx| Workspace::new(window, cx)),
            persistence: None,
            _observe_restore: None,
        }
    }

    /// The title bar over a new workspace whose saved layout `store` restores in the background
    /// and keeps saved (see the [module docs](self) for the placeholder it shows meanwhile).
    pub fn restoring(store: LayoutStore, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        let persistence = LayoutPersistence::start(&workspace, store, window, cx);
        // The controller notifies when the restore finishes: redraw without the marker.
        let observe = cx.observe(&persistence, |_, _, cx| cx.notify());
        Self {
            workspace,
            persistence: Some(persistence),
            _observe_restore: Some(observe),
        }
    }

    /// The window's workspace.
    pub fn workspace(&self) -> &Entity<Workspace> {
        &self.workspace
    }

    /// The layout persistence controller, when the window restores and saves its layout.
    pub fn persistence(&self) -> Option<&Entity<LayoutPersistence>> {
        self.persistence.as_ref()
    }

    /// Whether the saved layout is still being read (the placeholder is showing).
    pub fn is_restoring(&self, cx: &App) -> bool {
        self.persistence
            .as_ref()
            .is_some_and(|persistence| persistence.read(cx).is_restoring())
    }
}

impl Render for MainView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !super::menus::installed(cx) {
            // After this frame is drawn and presented: the menu bar is not on the path to it.
            cx.defer(super::menus::install_once);
        }
        let colors = cx.colors();
        let restoring = self.is_restoring(cx);
        v_flex()
            .id("main-view")
            .size_full()
            .bg(colors.background)
            .text_color(colors.text)
            .child(
                TitleBar::new()
                    .child(
                        div()
                            .id("window-title")
                            .debug_selector(|| "window-title".to_owned())
                            .text_sm()
                            .text_color(colors.text_muted)
                            .px(px(8.))
                            .child(WINDOW_TITLE),
                    )
                    .when(restoring, |bar| {
                        bar.child(
                            div()
                                .id("layout-restoring")
                                .debug_selector(|| "layout-restoring".to_owned())
                                .text_xs()
                                .text_color(colors.text_muted)
                                .px(px(12.))
                                .child(RESTORING_LABEL),
                        )
                    }),
            )
            .child(
                div()
                    .id("main-body")
                    .flex_1()
                    .min_h_0()
                    .child(self.workspace.clone()),
            )
    }
}
