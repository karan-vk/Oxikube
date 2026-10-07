//! [`ItemHandle`]: the object-safe face of an [`Item`] entity.

use gpui::{AnyView, App, Entity, EntityId, FocusHandle, SharedString, Subscription, Window};

use super::{CloseRequest, Item, ItemEvent, TabContent};

/// An [`Item`] entity of any type. Implemented for every `Entity<T: Item>`.
///
/// The workspace stores items as `Box<dyn ItemHandle>`; get the concrete entity back with
/// `downcast` (on `dyn ItemHandle`).
pub trait ItemHandle: 'static {
    /// The item's entity id: its identity in the workspace for as long as it is open.
    fn item_id(&self) -> EntityId;
    /// The item as a view.
    fn to_any_view(&self) -> AnyView;
    /// A second handle on the same item.
    fn boxed_clone(&self) -> Box<dyn ItemHandle>;
    /// See [`Item::tab_content`].
    fn tab_content(&self, cx: &App) -> TabContent;
    /// See [`Item::item_key`].
    fn item_key(&self, cx: &App) -> Option<SharedString>;
    /// See [`Item::can_close`].
    fn can_close(&self, cx: &App) -> bool;
    /// See [`Item::intercepts_close`].
    fn intercepts_close(&self, cx: &App) -> bool;
    /// See [`Item::close_requested`].
    fn close_requested(&self, window: &mut Window, cx: &mut App) -> CloseRequest;
    /// See [`Item::on_close`].
    fn on_close(&self, window: &mut Window, cx: &mut App);
    /// See [`Item::set_active`].
    fn set_active(&self, active: bool, window: &mut Window, cx: &mut App);
    /// The item's focus handle.
    fn focus_handle(&self, cx: &App) -> FocusHandle;
    /// See [`Item::can_dock`].
    fn can_dock(&self, cx: &App) -> bool;
    /// See [`Item::clone_on_split`].
    fn clone_on_split(&self, window: &mut Window, cx: &mut App) -> Option<Box<dyn ItemHandle>>;
    /// See [`Item::serialized_kind`].
    fn serialized_kind(&self) -> Option<&'static str>;
    /// See [`Item::serialize`].
    fn serialize(&self, cx: &App) -> Option<serde_json::Value>;
    /// Calls `handler` with every [`ItemEvent`] the item emits, until the subscription drops.
    fn subscribe_to_item_events(
        &self,
        window: &mut Window,
        cx: &mut App,
        handler: Box<dyn Fn(&ItemEvent, &mut Window, &mut App)>,
    ) -> Subscription;
}

impl<T: Item> ItemHandle for Entity<T> {
    fn item_id(&self) -> EntityId {
        self.entity_id()
    }

    fn to_any_view(&self) -> AnyView {
        self.clone().into()
    }

    fn boxed_clone(&self) -> Box<dyn ItemHandle> {
        Box::new(self.clone())
    }

    fn tab_content(&self, cx: &App) -> TabContent {
        self.read(cx).tab_content(cx)
    }

    fn item_key(&self, cx: &App) -> Option<SharedString> {
        self.read(cx).item_key(cx)
    }

    fn can_close(&self, cx: &App) -> bool {
        self.read(cx).can_close(cx)
    }

    fn intercepts_close(&self, cx: &App) -> bool {
        self.read(cx).intercepts_close(cx)
    }

    fn close_requested(&self, window: &mut Window, cx: &mut App) -> CloseRequest {
        self.update(cx, |item, cx| item.close_requested(window, cx))
    }

    fn on_close(&self, window: &mut Window, cx: &mut App) {
        self.update(cx, |item, cx| item.on_close(window, cx));
    }

    fn set_active(&self, active: bool, window: &mut Window, cx: &mut App) {
        self.update(cx, |item, cx| item.set_active(active, window, cx));
    }

    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.read(cx).focus_handle(cx)
    }

    fn can_dock(&self, cx: &App) -> bool {
        self.read(cx).can_dock(cx)
    }

    fn clone_on_split(&self, window: &mut Window, cx: &mut App) -> Option<Box<dyn ItemHandle>> {
        self.update(cx, |item, cx| item.clone_on_split(window, cx))
            .map(|clone| Box::new(clone) as Box<dyn ItemHandle>)
    }

    fn serialized_kind(&self) -> Option<&'static str> {
        T::serialized_kind()
    }

    fn serialize(&self, cx: &App) -> Option<serde_json::Value> {
        self.read(cx).serialize(cx)
    }

    fn subscribe_to_item_events(
        &self,
        window: &mut Window,
        cx: &mut App,
        handler: Box<dyn Fn(&ItemEvent, &mut Window, &mut App)>,
    ) -> Subscription {
        window.subscribe(self, cx, move |_, event: &ItemEvent, window, cx| {
            handler(event, window, cx)
        })
    }
}

impl dyn ItemHandle {
    /// The concrete entity behind this handle, if it is a `T`.
    pub fn downcast<T: Item>(&self) -> Option<Entity<T>> {
        self.to_any_view().downcast::<T>().ok()
    }
}

impl Clone for Box<dyn ItemHandle> {
    fn clone(&self) -> Self {
        self.boxed_clone()
    }
}

impl std::fmt::Debug for dyn ItemHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ItemHandle")
            .field("item_id", &self.item_id())
            .finish()
    }
}
