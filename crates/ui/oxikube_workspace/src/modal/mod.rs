//! The modal layer: one modal view at a time, over the whole workspace.
//!
//! Zed's `ModalLayer` design (written from scratch). The layer hosts a [`ModalView`]; opening a
//! second modal replaces the first. A modal closes on Escape (`Cancel`), on a click outside it
//! (the scrim), or when the view emits `DismissEvent`; the view can veto with
//! [`ModalView::on_before_dismiss`].
//!
//! Focus model (the story's "focus handles + tab stops"):
//!
//! - opening a modal remembers the element that had focus and focuses the modal's own handle;
//! - Tab and Shift-Tab cycle through the tab stops inside the modal only (see `focus`);
//! - closing the modal puts focus back on the remembered element, so the layer never keeps
//!   focus. When a replaced modal was the one that remembered it, the new modal inherits the
//!   memory, so the original element still gets focus back at the end.
//!
//! The layer is not gpui-component's dialog layer: `Root` renders that one, and rendering a second
//! set of overlay layers would stack two scrims. Dialog-shaped content is built with
//! [`DialogModal`] from `oxikube_ui`'s dialog pieces and buttons.

mod dialog;
mod focus;
mod view;

use std::any::TypeId;

use gpui::{
    App, AppContext as _, Context, DismissEvent, Empty, Entity, EventEmitter, FocusHandle,
    Focusable, InteractiveElement as _, IntoElement, KeyBinding, ParentElement as _, Render,
    Styled as _, Subscription, WeakFocusHandle, Window, actions, div, prelude::FluentBuilder as _,
    px,
};
use oxikube_ui::{ActiveTokens as _, dialog::Cancel, layout::h_flex, u};

pub use dialog::DialogModal;
pub use view::{ModalPlacement, ModalView, ModalViewHandle};

/// The key context the layer sets while a modal is open.
pub const MODAL_KEY_CONTEXT: &str = "ModalLayer";

actions!(
    modal,
    [
        /// Move focus to the next tab stop inside the open modal.
        FocusNext,
        /// Move focus to the previous tab stop inside the open modal.
        FocusPrev,
    ]
);

/// Registers the modal layer's key bindings: Escape closes, Tab / Shift-Tab cycle inside.
/// Called by [`crate::init`].
pub fn register(cx: &mut App) {
    dialog::register(cx);
    let ctx = Some(MODAL_KEY_CONTEXT);
    cx.bind_keys([
        KeyBinding::new("escape", Cancel, ctx),
        KeyBinding::new("tab", FocusNext, ctx),
        KeyBinding::new("shift-tab", FocusPrev, ctx),
    ]);
}

/// What the layer reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModalLayerEvent {
    /// A modal opened (or replaced the previous one).
    Shown,
    /// The open modal closed and nothing replaced it.
    Hidden,
}

struct ActiveModal {
    view: Box<dyn ModalViewHandle>,
    /// What had focus before the first modal of this run opened.
    previous_focus: Option<WeakFocusHandle>,
    _subscription: Subscription,
}

/// The modal layer entity. See the [module docs](self).
pub struct ModalLayer {
    active: Option<ActiveModal>,
    /// Wraps the open modal: it is what "focus is inside the modal" means.
    focus_handle: FocusHandle,
}

impl EventEmitter<ModalLayerEvent> for ModalLayer {}

impl ModalLayer {
    /// A layer with no modal open.
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            active: None,
            focus_handle: cx.focus_handle(),
        }
    }

    /// Opens a `V` built by `build`, or closes the open one when it already is a `V`. Any other
    /// open modal is replaced.
    pub fn toggle_modal<V: ModalView>(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        build: impl FnOnce(&mut Window, &mut Context<V>) -> V,
    ) {
        if self.active_is::<V>() {
            self.hide_modal(window, cx);
        } else {
            let view = cx.new(|cx| build(window, cx));
            self.show_modal(view, window, cx);
        }
    }

    /// Opens `view`, replacing the open modal without asking it ([`ModalView::on_before_dismiss`]
    /// is for the user's dismissal, not for being replaced).
    pub fn show_modal<V: ModalView>(
        &mut self,
        view: Entity<V>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let previous_focus = match self.active.take() {
            // Replacing: the new modal inherits the element to give focus back to.
            Some(active) => active.previous_focus,
            None => window.focused(cx).map(|handle| handle.downgrade()),
        };
        let subscription =
            cx.subscribe_in(&view, window, |this, _, _: &DismissEvent, window, cx| {
                this.hide_modal(window, cx);
            });
        let focus = Focusable::focus_handle(&view, cx);
        self.active = Some(ActiveModal {
            view: Box::new(view),
            previous_focus,
            _subscription: subscription,
        });
        focus.focus(window, cx);
        cx.emit(ModalLayerEvent::Shown);
        cx.notify();
    }

    /// Closes the open modal, unless it vetoes ([`ModalView::on_before_dismiss`]). Focus goes
    /// back to the element that had it before the modal opened. Returns whether a modal closed.
    pub fn hide_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(active) = &self.active else {
            return false;
        };
        if !active.view.on_before_dismiss(window, cx) {
            return false;
        }
        // Read focus before the modal leaves: focus inside it (or nowhere) is ours to hand back;
        // focus somewhere else was put there deliberately.
        let restore =
            self.focus_handle.contains_focused(window, cx) || window.focused(cx).is_none();
        let Some(active) = self.active.take() else {
            return false;
        };
        if restore && let Some(previous) = active.previous_focus.and_then(|weak| weak.upgrade()) {
            // After the current event: the click that dismissed the modal also focuses the scrim
            // (it is focusable), and that must not win over giving focus back.
            window.defer(cx, move |window, cx| previous.focus(window, cx));
        }
        cx.emit(ModalLayerEvent::Hidden);
        cx.notify();
        true
    }

    /// Whether a modal is open.
    pub fn has_active_modal(&self) -> bool {
        self.active.is_some()
    }

    /// The open modal, when it is a `V`.
    pub fn active_modal<V: ModalView>(&self) -> Option<Entity<V>> {
        self.active
            .as_ref()
            .and_then(|active| active.view.to_any_view().downcast::<V>().ok())
    }

    fn active_is<V: ModalView>(&self) -> bool {
        self.active
            .as_ref()
            .is_some_and(|active| active.view.view_type() == TypeId::of::<V>())
    }

    /// Whether focus is somewhere inside the open modal.
    pub fn is_focused(&self, window: &Window, cx: &App) -> bool {
        self.active.is_some() && self.focus_handle.contains_focused(window, cx)
    }

    /// Tab / Shift-Tab: stay inside the modal.
    fn cycle_focus(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(active) = &self.active else {
            return;
        };
        let modal = active.view.focus_handle(cx);
        if !focus::cycle(&self.focus_handle, forward, window, cx) {
            // No tab stop in the modal at all: keep focus on the modal itself.
            modal.focus(window, cx);
        }
    }
}

impl Focusable for ModalLayer {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ModalLayer {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(active) = &self.active else {
            return Empty.into_any_element();
        };
        let view = active.view.to_any_view();
        let dim = active.view.dim_background(cx);
        let outside_click = active.view.dismiss_on_outside_click(cx);
        let center = active.view.placement(cx) == ModalPlacement::Center;
        let scrim = cx.colors().background.opacity(0.55);

        div()
            .id("modal-layer")
            .debug_selector(|| "modal-layer".to_owned())
            .key_context(MODAL_KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .tab_group()
            .absolute()
            .inset_0()
            .occlude()
            .flex()
            .flex_col()
            .items_center()
            .when(dim, |this| this.bg(scrim))
            .when(center, |this| this.justify_center())
            .when(!center, |this| this.pt(u(px(80.))))
            .on_action(cx.listener(|this, _: &Cancel, window, cx| {
                this.hide_modal(window, cx);
            }))
            .on_action(cx.listener(|this, _: &FocusNext, window, cx| {
                this.cycle_focus(true, window, cx);
            }))
            .on_action(cx.listener(|this, _: &FocusPrev, window, cx| {
                this.cycle_focus(false, window, cx);
            }))
            .child(
                h_flex()
                    .id("modal-content")
                    .debug_selector(|| "modal-content".to_owned())
                    .occlude()
                    .on_mouse_down_out(cx.listener(move |this, _, window, cx| {
                        if outside_click {
                            this.hide_modal(window, cx);
                        }
                    }))
                    .child(view),
            )
            .into_any_element()
    }
}
