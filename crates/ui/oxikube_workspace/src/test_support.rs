//! A test item and a test panel for `#[gpui::test]`s of the workspace and of the crates built on
//! it (feature `test-support`).

use std::{cell::Cell, rc::Rc};

use gpui::{
    Action, App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement as _, Pixels, Render, SharedString,
    Styled as _, Window, actions, div, px,
};
use oxikube_ui::IconName;

use crate::{
    item::{Item, ItemEvent, TabContent, register_item},
    panel::{DockPosition, Panel, PanelEvent},
};

actions!(
    test_support,
    [
        /// Toggles the left [`TestPanel`].
        ToggleLeftTestPanel,
        /// Toggles the right [`TestPanel`].
        ToggleRightTestPanel,
        /// Toggles the bottom [`TestPanel`].
        ToggleBottomTestPanel,
    ]
);

/// A centre item with a title, an optional key, and switches for closing and splitting.
///
/// It renders a full-size body tagged `item-<title>` for `debug_bounds`; its tab label is tagged
/// `tab-<title>`.
pub struct TestItem {
    focus_handle: FocusHandle,
    /// The tab title (also its serialised state).
    pub title: SharedString,
    /// The dedup key.
    pub key: Option<SharedString>,
    /// Whether [`Item::can_close`] allows closing.
    pub closable: bool,
    /// Whether [`Item::clone_on_split`] makes a copy.
    pub cloneable: bool,
    /// The dirty flag.
    pub dirty: bool,
    /// How many times [`Item::on_close`] ran.
    pub closed: Rc<Cell<usize>>,
    /// The last [`Item::set_active`] value.
    pub active: bool,
}

impl TestItem {
    /// The serialised kind of test items.
    pub const KIND: &'static str = "test_support::TestItem";

    /// A closable, non-cloneable item titled `title`.
    pub fn new(title: impl Into<SharedString>, cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            title: title.into(),
            key: None,
            closable: true,
            cloneable: false,
            dirty: false,
            closed: Rc::default(),
            active: false,
        }
    }

    /// Builds a test item entity.
    pub fn build(title: impl Into<SharedString>, cx: &mut App) -> Entity<Self> {
        let title = title.into();
        cx.new(|cx| Self::new(title, cx))
    }

    /// Sets the dedup key.
    pub fn with_key(mut self, key: impl Into<SharedString>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Makes [`Item::clone_on_split`] copy the item.
    pub fn cloneable(mut self) -> Self {
        self.cloneable = true;
        self
    }

    /// Sets the title and tells the workspace to redraw the tab.
    pub fn set_title(&mut self, title: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.title = title.into();
        cx.emit(ItemEvent::UpdateTab);
    }
}

/// Registers the [`TestItem`] builder so closed test items can be reopened.
pub fn register_test_item(cx: &mut App) {
    register_item::<TestItem>(cx, |state, _, cx| {
        let title = state.as_str()?.to_owned();
        Some(TestItem::build(title, cx))
    });
}

impl EventEmitter<ItemEvent> for TestItem {}

impl Focusable for TestItem {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for TestItem {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let selector = format!("item-{}", self.title);
        div()
            .id("test-item")
            .debug_selector(move || selector)
            .track_focus(&self.focus_handle)
            .size_full()
            .child(self.title.clone())
    }
}

impl Item for TestItem {
    fn tab_content(&self, _: &App) -> TabContent {
        TabContent::new(self.title.clone())
            .icon(IconName::FileText)
            .dirty(self.dirty)
    }

    fn item_key(&self, _: &App) -> Option<SharedString> {
        self.key.clone()
    }

    fn can_close(&self, _: &App) -> bool {
        self.closable
    }

    fn on_close(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.closed.set(self.closed.get() + 1);
    }

    fn set_active(&mut self, active: bool, _: &mut Window, _: &mut Context<Self>) {
        self.active = active;
    }

    fn clone_on_split(&self, _: &mut Window, cx: &mut Context<Self>) -> Option<Entity<Self>> {
        if !self.cloneable {
            return None;
        }
        let title = self.title.clone();
        Some(cx.new(|cx| Self::new(title, cx).cloneable()))
    }

    fn serialized_kind() -> Option<&'static str> {
        Some(Self::KIND)
    }

    fn serialize(&self, _: &App) -> Option<serde_json::Value> {
        Some(serde_json::Value::String(self.title.to_string()))
    }
}

/// A side panel at a chosen dock position. Renders a full-size body tagged `panel-<title>`.
pub struct TestPanel {
    focus_handle: FocusHandle,
    /// The dock it asks for.
    pub position: DockPosition,
    /// The tab title.
    pub title: SharedString,
    /// Its activation priority.
    pub priority: u32,
    /// Its minimum size, unscaled.
    pub min_size: Option<Pixels>,
    /// The last [`Panel::set_active`] value.
    pub active: bool,
    /// The last [`Panel::set_zoomed`] value.
    pub zoomed: bool,
}

impl TestPanel {
    /// A panel titled `title` in the dock at `position`.
    pub fn build(position: DockPosition, title: &str, cx: &mut App) -> Entity<Self> {
        let title = SharedString::from(title.to_owned());
        cx.new(|cx| Self {
            focus_handle: cx.focus_handle(),
            position,
            title,
            priority: 0,
            min_size: None,
            active: false,
            zoomed: false,
        })
    }

    /// Asks the workspace to show and focus this panel.
    pub fn activate(&mut self, cx: &mut Context<Self>) {
        cx.emit(PanelEvent::Activate);
    }
}

impl EventEmitter<PanelEvent> for TestPanel {}

impl Focusable for TestPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for TestPanel {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let selector = format!("panel-{}", self.title);
        div()
            .id("test-panel")
            .debug_selector(move || selector)
            .track_focus(&self.focus_handle)
            .size_full()
            .child(self.title.clone())
    }
}

impl Panel for TestPanel {
    fn persistent_name() -> &'static str {
        "TestPanel"
    }

    fn panel_key() -> &'static str {
        "test_panel"
    }

    fn position(&self, _: &Window, _: &App) -> DockPosition {
        self.position
    }

    fn default_size(&self, _: &Window, _: &App) -> Pixels {
        px(240.)
    }

    fn min_size(&self, _: &Window, _: &App) -> Option<Pixels> {
        self.min_size
    }

    fn icon(&self, _: &Window, _: &App) -> Option<IconName> {
        Some(IconName::PanelLeft)
    }

    fn title(&self, _: &App) -> SharedString {
        self.title.clone()
    }

    fn toggle_action(&self) -> Box<dyn Action> {
        match self.position {
            DockPosition::Left => Box::new(ToggleLeftTestPanel),
            DockPosition::Right => Box::new(ToggleRightTestPanel),
            DockPosition::Bottom => Box::new(ToggleBottomTestPanel),
        }
    }

    fn activation_priority(&self) -> u32 {
        self.priority
    }

    fn set_active(&mut self, active: bool, _: &mut Window, _: &mut Context<Self>) {
        self.active = active;
    }

    fn set_zoomed(&mut self, zoomed: bool, _: &mut Window, _: &mut Context<Self>) {
        self.zoomed = zoomed;
    }
}
