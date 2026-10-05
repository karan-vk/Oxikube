//! The overlay surfaces every feature shares: the status bar, the modal layer and the toast
//! layer. The workspace owns one of each and draws them over the docks (see `render`); feature
//! code reaches them through these methods and never builds its own.

use gpui::{Context, Entity, Window};

use super::Workspace;
use crate::{
    modal::{ModalLayer, ModalView},
    status_bar::{StatusBar, StatusItem, StatusItemId, StatusSide},
    toast::{Toast, ToastId, ToastLayer},
};

impl Workspace {
    /// The status bar entity.
    pub fn status_bar(&self) -> &Entity<StatusBar> {
        &self.status_bar
    }

    /// The modal layer entity.
    pub fn modal_layer(&self) -> &Entity<ModalLayer> {
        &self.modal_layer
    }

    /// The toast layer entity.
    pub fn toast_layer(&self) -> &Entity<ToastLayer> {
        &self.toast_layer
    }

    /// Adds `view` to the status bar's `side` at `priority` (lower is further left).
    pub fn register_status_item<V: StatusItem>(
        &mut self,
        side: StatusSide,
        priority: i32,
        view: Entity<V>,
        cx: &mut Context<Self>,
    ) -> StatusItemId {
        self.status_bar.update(cx, |bar, cx| {
            bar.register_status_item(side, priority, view, cx)
        })
    }

    /// Opens a modal `V` built by `build`, or closes the open one when it already is a `V`. A
    /// different open modal is replaced.
    pub fn toggle_modal<V: ModalView>(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        build: impl FnOnce(&mut Window, &mut Context<V>) -> V,
    ) {
        self.modal_layer
            .update(cx, |layer, cx| layer.toggle_modal(window, cx, build));
    }

    /// Opens `view` as the modal, replacing the open one.
    pub fn show_modal<V: ModalView>(
        &mut self,
        view: Entity<V>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.modal_layer
            .update(cx, |layer, cx| layer.show_modal(view, window, cx));
    }

    /// Closes the open modal (unless it vetoes). Returns whether one closed.
    pub fn hide_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.modal_layer
            .update(cx, |layer, cx| layer.hide_modal(window, cx))
    }

    /// Shows a toast (or updates the one with the same key).
    pub fn show_toast(&mut self, toast: Toast, cx: &mut Context<Self>) -> ToastId {
        self.toast_layer
            .update(cx, |layer, cx| layer.show(toast, cx))
    }
}
