//! Test doubles for the overlay layers: a status item and a modal view.

use std::{cell::Cell, rc::Rc};

use gpui::{
    App, Context, DismissEvent, EventEmitter, FocusHandle, Focusable, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, SharedString, Styled as _, Window, div, px,
};

use crate::{modal::ModalView, status_bar::StatusItem};

/// A status item that shows a label. Renders a body tagged `status-<label>`.
pub struct TestStatusItem {
    /// The text shown.
    pub label: SharedString,
    /// Whether [`StatusItem::visible`] says yes.
    pub visible: bool,
}

impl TestStatusItem {
    /// A visible item showing `label`.
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            visible: true,
        }
    }

    /// Shows or hides the item and tells the bar.
    pub fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        self.visible = visible;
        cx.notify();
    }

    /// Changes the label and tells the bar.
    pub fn set_label(&mut self, label: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.label = label.into();
        cx.notify();
    }
}

impl Render for TestStatusItem {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let selector = format!("status-{}", self.label);
        div()
            .id("test-status-item")
            .debug_selector(move || selector)
            .child(self.label.clone())
    }
}

impl StatusItem for TestStatusItem {
    fn visible(&self, _: &App) -> bool {
        self.visible
    }
}

/// A modal with two tab stops (`first`, `second`). Renders a body tagged `modal-<title>`.
pub struct TestModal {
    focus_handle: FocusHandle,
    /// First tab stop inside the modal.
    pub first: FocusHandle,
    /// Second tab stop inside the modal.
    pub second: FocusHandle,
    /// The title (in the selector).
    pub title: SharedString,
    /// Whether [`ModalView::on_before_dismiss`] vetoes.
    pub veto: bool,
    /// How many times [`ModalView::on_before_dismiss`] was asked.
    pub asked: Rc<Cell<usize>>,
}

impl TestModal {
    /// A modal titled `title` that lets itself be dismissed.
    pub fn new(title: impl Into<SharedString>, cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            first: cx.focus_handle().tab_stop(true),
            second: cx.focus_handle().tab_stop(true),
            title: title.into(),
            veto: false,
            asked: Rc::default(),
        }
    }

    /// Asks the layer to close this modal, as a view's own "Close" button would.
    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        cx.emit(DismissEvent);
    }
}

impl EventEmitter<DismissEvent> for TestModal {}

impl Focusable for TestModal {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl ModalView for TestModal {
    fn on_before_dismiss(&mut self, _: &mut Window, _: &mut App) -> bool {
        self.asked.set(self.asked.get() + 1);
        !self.veto
    }
}

impl Render for TestModal {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let selector = format!("modal-{}", self.title);
        div()
            .id("test-modal")
            .debug_selector(move || selector)
            .track_focus(&self.focus_handle)
            .tab_group()
            .w(px(300.))
            .h(px(120.))
            .child(
                div()
                    .id("modal-first")
                    .track_focus(&self.first)
                    .w_full()
                    .h(px(40.)),
            )
            .child(
                div()
                    .id("modal-second")
                    .track_focus(&self.second)
                    .w_full()
                    .h(px(40.)),
            )
    }
}
