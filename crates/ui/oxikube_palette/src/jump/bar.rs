//! [`JumpBar`]: the modal view around the jump bar's [`Picker`]. It carries the `JumpBar` key
//! context (the picker's own `Picker` / `Picker > Input` contexts sit inside it), so Tab
//! (`jump_bar::Complete`) lives in `JumpBar` and the list navigation in `Picker`; printable keys,
//! `:` included, reach the query field.
//!
//! When the bar opens it asks the Tokio bridge for the cluster contexts and the shown cluster's
//! namespaces. The answer replaces the snapshot the line is checked against and refreshes the
//! completions; the first frame never waits for it.

use std::rc::Rc;

use gpui::{
    App, AppContext as _, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, Subscription, Task, Window,
    div,
};
use oxikube_keymap::{KeyContextual, contexts};
use oxikube_runtime::spawn_kube;
use oxikube_workspace::modal::{ModalPlacement, ModalView};

use super::Complete;
use super::delegate::JumpDelegate;
use super::host::Shared;
use super::sources::{JumpSources, LiveEnv, Loaded};
use crate::picker::Picker;

/// The width of the bar (before UI zoom): wide enough for `deploy kube-system /api app=x @prod`.
const WIDTH: gpui::Pixels = gpui::px(640.);

/// The `:` jump bar as a modal. Open it with [`JumpHost::open`](super::JumpHost::open).
pub struct JumpBar {
    picker: Entity<Picker<JumpDelegate>>,
    sources: JumpSources,
    shared: Rc<Shared>,
    /// The read of the contexts and namespaces, cancelled when the bar closes.
    _load: Task<()>,
    _dismiss: Subscription,
}

impl JumpBar {
    pub(super) fn new(
        delegate: JumpDelegate,
        sources: JumpSources,
        shared: Rc<Shared>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let picker = cx.new(|cx| Picker::uniform_list(delegate, window, cx).width(WIDTH));
        // The picker closes itself (confirm, Escape); the modal layer listens to this view.
        let dismiss = cx.subscribe(&picker, |_, _, _: &DismissEvent, cx| cx.emit(DismissEvent));

        let active = (sources.active)(cx);
        let load = spawn_kube(
            cx,
            Loaded::read(sources.catalog.clone(), sources.namespaces.clone(), active),
        );
        let loading = cx.spawn_in(window, async move |this, cx| {
            let Ok(loaded) = load.await else {
                return;
            };
            this.update_in(cx, |bar, window, cx| bar.loaded(loaded, window, cx))
                .ok();
        });
        Self {
            picker,
            sources,
            shared,
            _load: loading,
            _dismiss: dismiss,
        }
    }

    /// The picker, for tests and the host.
    pub fn picker(&self) -> &Entity<Picker<JumpDelegate>> {
        &self.picker
    }

    /// The contexts and namespaces arrived: keep them for the next open, and check the line
    /// against them from now on.
    fn loaded(&mut self, loaded: Loaded, window: &mut Window, cx: &mut Context<Self>) {
        self.shared.loaded.borrow_mut().merge(loaded);
        let env = LiveEnv::snapshot(&self.sources, &self.shared.loaded.borrow(), cx);
        self.picker.update(cx, |picker, cx| {
            picker.delegate.set_env(Rc::new(env));
            picker.refresh(window, cx);
        });
    }

    /// Tab: replaces the word being typed by the selected completion. While the completions of
    /// the newest line are still being matched (a cluster with many kinds), it waits for them
    /// instead of completing from the previous line's.
    pub fn complete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.picker.read(cx).is_matching() {
            self.picker
                .update(cx, |picker, _| picker.delegate.complete_when_matched());
            return;
        }
        let Some((line, site, text)) = self.picker.read(cx).delegate.completion() else {
            return;
        };
        let line = oxikube_app::search::jump::accept(&line, &site, &text);
        self.picker
            .update(cx, |picker, cx| picker.set_query(&line, window, cx));
    }
}

impl EventEmitter<DismissEvent> for JumpBar {}

impl Focusable for JumpBar {
    /// The query field: the bar opens with the caret there.
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.picker.focus_handle(cx)
    }
}

impl KeyContextual for JumpBar {
    const KEY_CONTEXT: &'static str = contexts::JUMP_BAR;
}

impl ModalView for JumpBar {
    fn on_before_dismiss(&mut self, window: &mut Window, cx: &mut App) -> bool {
        self.picker
            .update(cx, |picker, cx| picker.on_before_dismiss(window, cx))
    }

    /// A command line, not a dialog: the view under it stays readable.
    fn dim_background(&self, _: &App) -> bool {
        false
    }

    fn placement(&self, _: &App) -> ModalPlacement {
        ModalPlacement::Top
    }
}

impl Render for JumpBar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("jump-bar")
            .key_context(self.key_context())
            .on_action(cx.listener(|this, _: &Complete, window, cx| this.complete(window, cx)))
            .child(self.picker.clone())
    }
}
