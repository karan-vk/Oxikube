//! [`PanelHandle`]: the object-safe face of a [`Panel`] entity.

use gpui::{
    Action, AnyView, App, Entity, EntityId, FocusHandle, Pixels, SharedString, Subscription, Window,
};
use oxikube_ui::IconName;

use super::{DockPosition, Panel, PanelEvent};

/// A [`Panel`] entity of any type. Implemented for every `Entity<T: Panel>`.
pub trait PanelHandle: 'static {
    /// The panel's entity id.
    fn panel_id(&self) -> EntityId;
    /// See [`Panel::persistent_name`].
    fn persistent_name(&self) -> &'static str;
    /// See [`Panel::panel_key`].
    fn panel_key(&self) -> &'static str;
    /// See [`Panel::position`].
    fn position(&self, window: &Window, cx: &App) -> DockPosition;
    /// See [`Panel::default_size`].
    fn default_size(&self, window: &Window, cx: &App) -> Pixels;
    /// See [`Panel::min_size`].
    fn min_size(&self, window: &Window, cx: &App) -> Option<Pixels>;
    /// See [`Panel::icon`].
    fn icon(&self, window: &Window, cx: &App) -> Option<IconName>;
    /// See [`Panel::icon_tooltip`].
    fn icon_tooltip(&self, window: &Window, cx: &App) -> Option<SharedString>;
    /// See [`Panel::title`].
    fn title(&self, cx: &App) -> SharedString;
    /// See [`Panel::toggle_action`].
    fn toggle_action(&self, cx: &App) -> Box<dyn Action>;
    /// See [`Panel::activation_priority`].
    fn activation_priority(&self, cx: &App) -> u32;
    /// See [`Panel::set_active`].
    fn set_active(&self, active: bool, window: &mut Window, cx: &mut App);
    /// See [`Panel::set_zoomed`].
    fn set_zoomed(&self, zoomed: bool, window: &mut Window, cx: &mut App);
    /// See [`Panel::serialize`].
    fn serialize(&self, cx: &App) -> Option<serde_json::Value>;
    /// The panel's focus handle.
    fn focus_handle(&self, cx: &App) -> FocusHandle;
    /// The panel as a view.
    fn to_any_view(&self) -> AnyView;
    /// A second handle on the same panel.
    fn boxed_clone(&self) -> Box<dyn PanelHandle>;
    /// Calls `handler` with every [`PanelEvent`] the panel emits, until the subscription drops.
    fn subscribe_to_panel_events(
        &self,
        window: &mut Window,
        cx: &mut App,
        handler: Box<dyn Fn(&PanelEvent, &mut Window, &mut App)>,
    ) -> Subscription;
}

impl<T: Panel> PanelHandle for Entity<T> {
    fn panel_id(&self) -> EntityId {
        self.entity_id()
    }

    fn persistent_name(&self) -> &'static str {
        T::persistent_name()
    }

    fn panel_key(&self) -> &'static str {
        T::panel_key()
    }

    fn position(&self, window: &Window, cx: &App) -> DockPosition {
        self.read(cx).position(window, cx)
    }

    fn default_size(&self, window: &Window, cx: &App) -> Pixels {
        self.read(cx).default_size(window, cx)
    }

    fn min_size(&self, window: &Window, cx: &App) -> Option<Pixels> {
        self.read(cx).min_size(window, cx)
    }

    fn icon(&self, window: &Window, cx: &App) -> Option<IconName> {
        self.read(cx).icon(window, cx)
    }

    fn icon_tooltip(&self, window: &Window, cx: &App) -> Option<SharedString> {
        self.read(cx).icon_tooltip(window, cx)
    }

    fn title(&self, cx: &App) -> SharedString {
        self.read(cx).title(cx)
    }

    fn toggle_action(&self, cx: &App) -> Box<dyn Action> {
        self.read(cx).toggle_action()
    }

    fn activation_priority(&self, cx: &App) -> u32 {
        self.read(cx).activation_priority()
    }

    fn set_active(&self, active: bool, window: &mut Window, cx: &mut App) {
        self.update(cx, |panel, cx| panel.set_active(active, window, cx));
    }

    fn set_zoomed(&self, zoomed: bool, window: &mut Window, cx: &mut App) {
        self.update(cx, |panel, cx| panel.set_zoomed(zoomed, window, cx));
    }

    fn serialize(&self, cx: &App) -> Option<serde_json::Value> {
        self.read(cx).serialize(cx)
    }

    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.read(cx).focus_handle(cx)
    }

    fn to_any_view(&self) -> AnyView {
        self.clone().into()
    }

    fn boxed_clone(&self) -> Box<dyn PanelHandle> {
        Box::new(self.clone())
    }

    fn subscribe_to_panel_events(
        &self,
        window: &mut Window,
        cx: &mut App,
        handler: Box<dyn Fn(&PanelEvent, &mut Window, &mut App)>,
    ) -> Subscription {
        window.subscribe(self, cx, move |_, event: &PanelEvent, window, cx| {
            handler(event, window, cx)
        })
    }
}

impl dyn PanelHandle {
    /// The concrete entity behind this handle, if it is a `T`.
    pub fn downcast<T: Panel>(&self) -> Option<Entity<T>> {
        self.to_any_view().downcast::<T>().ok()
    }
}

impl Clone for Box<dyn PanelHandle> {
    fn clone(&self) -> Self {
        self.boxed_clone()
    }
}
