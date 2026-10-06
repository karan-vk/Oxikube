//! The toast layer: a short queue of transient messages in the bottom-right corner.
//!
//! - **Queue**: at most [`DEFAULT_MAX_VISIBLE`] toasts are on screen; the rest wait and slide in
//!   as others go (oldest first). See `model::ToastQueue`.
//! - **Deduplication**: a toast with a key ([`Toast::key`]) replaces the visible or waiting toast
//!   with the same key in place and restarts its timeout, so a retry loop does not stack ten
//!   identical errors.
//! - **Auto-dismiss**: each level has a default timeout ([`ToastLevel::default_timeout`]; errors
//!   stay); [`Toast::timeout`] and [`Toast::persistent`] override. Timers use GPUI's executor
//!   clock, so tests advance them with `cx.executor().advance_clock(..)`.
//! - **Actions**: [`ToastAction`] buttons ("Retry"); clicking one runs it and dismisses the toast.
//! - **Focus**: showing a toast never moves focus. Keyboard users call
//!   [`ToastLayer::focus_toasts`], which remembers the element that had focus; Escape on a toast
//!   dismisses it, and when the focused toast goes away (Escape, click, timeout) focus returns to
//!   the remembered element.
//! - **Layout**: the layer is absolutely positioned over the workspace, so a toast appearing or
//!   disappearing never lays out the rest of the window.
//!
//! Persisting notifications into a panel is later work (the notifications panel).

mod model;
mod view;

use std::collections::HashMap;

use gpui::{
    AnyWindowHandle, App, Context, EventEmitter, FocusHandle, Focusable, KeyBinding, SharedString,
    WeakFocusHandle, Window,
};
use oxikube_ui::dialog::Cancel;

use model::ToastQueue;
pub use model::{Toast, ToastAction, ToastId, ToastLevel};

/// How many toasts are on screen at once unless [`ToastLayer::set_max_visible`] says otherwise.
pub const DEFAULT_MAX_VISIBLE: usize = 3;

/// The key context of the layer.
pub const TOAST_KEY_CONTEXT: &str = "ToastLayer";

/// Registers Escape = dismiss in the toast layer's context. Called by [`crate::init`].
pub fn register(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("escape", Cancel, Some(TOAST_KEY_CONTEXT))]);
}

/// What the layer reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastLayerEvent {
    /// A toast left the queue (timeout, click, Escape or code).
    Dismissed(ToastId),
}

/// A read-only view of a visible toast.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToastSummary {
    /// The toast's id.
    pub id: ToastId,
    /// Its deduplication key.
    pub key: Option<SharedString>,
    /// Its level.
    pub level: ToastLevel,
    /// Its message.
    pub message: SharedString,
}

/// The toast layer entity. See the [module docs](self).
pub struct ToastLayer {
    queue: ToastQueue,
    /// The wrapper of all visible toasts; "focus is in the layer" means focus inside it.
    focus_handle: FocusHandle,
    /// One handle per visible toast, so each can be focused and dismissed on its own.
    toast_handles: HashMap<ToastId, FocusHandle>,
    /// What had focus when [`ToastLayer::focus_toasts`] took it.
    previous_focus: Option<WeakFocusHandle>,
    window: AnyWindowHandle,
}

impl EventEmitter<ToastLayerEvent> for ToastLayer {}

impl Focusable for ToastLayer {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl ToastLayer {
    /// An empty layer for `window`.
    pub fn new(window: &Window, cx: &mut Context<Self>) -> Self {
        Self {
            queue: ToastQueue::new(DEFAULT_MAX_VISIBLE),
            focus_handle: cx.focus_handle(),
            toast_handles: HashMap::new(),
            previous_focus: None,
            window: window.window_handle(),
        }
    }

    /// Shows `toast` (or updates the toast with the same key). Never moves focus.
    pub fn show(&mut self, toast: Toast, cx: &mut Context<Self>) -> ToastId {
        let pushed = self.queue.push(toast);
        if pushed.visible {
            self.started(pushed.id, cx);
        }
        cx.notify();
        pushed.id
    }

    /// Dismisses a toast, visible or waiting. Returns whether it existed.
    pub fn dismiss(&mut self, id: ToastId, cx: &mut Context<Self>) -> bool {
        let (existed, promoted) = self.queue.dismiss(id);
        self.removed(existed.then_some(id), promoted, cx);
        existed
    }

    /// Dismisses the toast with `key`. Returns whether there was one.
    pub fn dismiss_key(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        let (id, promoted) = self.queue.dismiss_key(key);
        let existed = id.is_some();
        self.removed(id, promoted, cx);
        existed
    }

    /// Changes how many toasts are visible at once (at least one).
    pub fn set_max_visible(&mut self, max: usize, cx: &mut Context<Self>) {
        let promoted = self.queue.set_max_visible(max);
        self.removed(None, promoted, cx);
    }

    /// How many toasts are visible at once.
    pub fn max_visible(&self) -> usize {
        self.queue.max_visible()
    }

    /// The visible toasts, oldest first.
    pub fn visible(&self) -> Vec<ToastSummary> {
        self.queue
            .visible
            .iter()
            .map(|entry| ToastSummary {
                id: entry.id,
                key: entry.toast.key.clone(),
                level: entry.toast.level,
                message: entry.toast.message.clone(),
            })
            .collect()
    }

    /// How many toasts wait for a free slot.
    pub fn pending_len(&self) -> usize {
        self.queue.pending_len()
    }

    /// Moves focus to the oldest visible toast so the keyboard can reach its buttons, remembering
    /// what had focus. Returns false when there is no toast.
    pub fn focus_toasts(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(first) = self.queue.visible.first() else {
            return false;
        };
        let Some(handle) = self.toast_handles.get(&first.id).cloned() else {
            return false;
        };
        if !self.focus_handle.contains_focused(window, cx) {
            self.previous_focus = window.focused(cx).map(|focused| focused.downgrade());
        }
        handle.focus(window, cx);
        true
    }

    /// Whether focus is on a toast.
    pub fn is_focused(&self, window: &Window, cx: &App) -> bool {
        self.focus_handle.contains_focused(window, cx)
    }

    /// A toast became visible: give it a focus handle and start its timeout.
    fn started(&mut self, id: ToastId, cx: &mut Context<Self>) {
        self.toast_handles
            .entry(id)
            .or_insert_with(|| cx.focus_handle().tab_stop(true));
        let Some(entry) = self.queue.get(id) else {
            return;
        };
        let (generation, Some(timeout)) = (entry.generation, entry.toast.timeout) else {
            return;
        };
        // Detached on purpose: a stale timer (the toast was dismissed or re-shown) finds a newer
        // generation, or nothing, and does nothing. Nothing is stored, so nothing drops itself.
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(timeout).await;
            this.update(cx, |this, cx| this.expire(id, generation, cx))
                .ok();
        })
        .detach();
    }

    fn expire(&mut self, id: ToastId, generation: u64, cx: &mut Context<Self>) {
        if self
            .queue
            .get(id)
            .is_some_and(|entry| entry.generation == generation)
        {
            self.dismiss(id, cx);
        }
    }

    /// After a removal: start the promoted toasts, hand focus back if the focused toast went.
    fn removed(
        &mut self,
        removed: Option<ToastId>,
        promoted: Vec<ToastId>,
        cx: &mut Context<Self>,
    ) {
        let handles: Vec<_> = removed
            .into_iter()
            .filter_map(|id| self.toast_handles.remove(&id))
            .collect();
        if let Some(id) = removed {
            cx.emit(ToastLayerEvent::Dismissed(id));
        }
        for id in promoted {
            self.started(id, cx);
        }
        self.restore_focus_after(handles, cx);
        cx.notify();
    }

    /// If one of the `removed` toast handles holds focus, gives focus back to the element that
    /// had it before [`ToastLayer::focus_toasts`]. Deferred: needs the window, and runs after the
    /// current update so it is safe from any caller.
    fn restore_focus_after(&mut self, removed: Vec<FocusHandle>, cx: &mut Context<Self>) {
        if self.previous_focus.is_none() || removed.is_empty() {
            return;
        }
        let window = self.window;
        let layer = cx.entity();
        cx.defer(move |cx| {
            window
                .update(cx, |_, window, cx| {
                    layer.update(cx, |this, cx| {
                        let lost = removed.iter().any(|handle| handle.is_focused(window));
                        if !lost {
                            return;
                        }
                        // Another toast takes the focus when one is left; otherwise it goes home.
                        if let Some(next) = this
                            .queue
                            .visible
                            .first()
                            .and_then(|entry| this.toast_handles.get(&entry.id))
                        {
                            next.focus(window, cx);
                        } else if let Some(previous) =
                            this.previous_focus.take().and_then(|weak| weak.upgrade())
                        {
                            previous.focus(window, cx);
                        }
                    });
                })
                .ok();
        });
    }
}
