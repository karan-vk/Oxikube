//! [`ModalView`]: what a view implements to be hosted by the [`ModalLayer`](super::ModalLayer),
//! and its object-safe handle.

use std::any::TypeId;

use gpui::{
    AnyView, App, DismissEvent, Entity, EntityId, EventEmitter, FocusHandle, Focusable, Render,
    Window,
};

/// Where the layer puts a modal in the window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ModalPlacement {
    /// Horizontally centred, near the top: pickers and palettes that grow downwards.
    #[default]
    Top,
    /// Centred both ways: confirmation dialogs.
    Center,
}

/// A view the modal layer can host: focusable, renderable, and able to ask for its own dismissal
/// by emitting [`DismissEvent`] (`cx.emit(DismissEvent)`).
///
/// The layer draws the scrim and positions the view; the view draws its own chrome. Escape,
/// a click outside and `DismissEvent` all close the modal through [`ModalView::on_before_dismiss`].
pub trait ModalView: Focusable + EventEmitter<DismissEvent> + Render + 'static {
    /// Asked before the modal closes (Escape, outside click, `DismissEvent`, replacement by
    /// another modal never asks). Return `false` to keep it open, for example while a form holds
    /// unsaved input; the view is then responsible for explaining why. Default: allow.
    fn on_before_dismiss(&mut self, _window: &mut Window, _cx: &mut App) -> bool {
        true
    }

    /// Whether a click on the scrim, outside the view, closes the modal. Default: yes.
    fn dismiss_on_outside_click(&self, _cx: &App) -> bool {
        true
    }

    /// Whether the layer dims the window behind the view. Default: yes.
    fn dim_background(&self, _cx: &App) -> bool {
        true
    }

    /// Where the layer puts the view. Default: [`ModalPlacement::Top`].
    fn placement(&self, _cx: &App) -> ModalPlacement {
        ModalPlacement::Top
    }
}

/// A [`ModalView`] entity of any type. Implemented for every `Entity<T: ModalView>`.
pub trait ModalViewHandle: 'static {
    /// The view's entity id.
    fn view_id(&self) -> EntityId;
    /// The concrete view type, to tell "the same modal again" from "another modal".
    fn view_type(&self) -> TypeId;
    /// The view as a view.
    fn to_any_view(&self) -> AnyView;
    /// The view's focus handle.
    fn focus_handle(&self, cx: &App) -> FocusHandle;
    /// See [`ModalView::on_before_dismiss`].
    fn on_before_dismiss(&self, window: &mut Window, cx: &mut App) -> bool;
    /// See [`ModalView::dismiss_on_outside_click`].
    fn dismiss_on_outside_click(&self, cx: &App) -> bool;
    /// See [`ModalView::dim_background`].
    fn dim_background(&self, cx: &App) -> bool;
    /// See [`ModalView::placement`].
    fn placement(&self, cx: &App) -> ModalPlacement;
}

impl<T: ModalView> ModalViewHandle for Entity<T> {
    fn view_id(&self) -> EntityId {
        self.entity_id()
    }

    fn view_type(&self) -> TypeId {
        TypeId::of::<T>()
    }

    fn to_any_view(&self) -> AnyView {
        self.clone().into()
    }

    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.read(cx).focus_handle(cx)
    }

    fn on_before_dismiss(&self, window: &mut Window, cx: &mut App) -> bool {
        self.update(cx, |view, cx| view.on_before_dismiss(window, cx))
    }

    fn dismiss_on_outside_click(&self, cx: &App) -> bool {
        self.read(cx).dismiss_on_outside_click(cx)
    }

    fn dim_background(&self, cx: &App) -> bool {
        self.read(cx).dim_background(cx)
    }

    fn placement(&self, cx: &App) -> ModalPlacement {
        self.read(cx).placement(cx)
    }
}
