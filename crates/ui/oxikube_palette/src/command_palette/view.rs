//! [`CommandPalette`]: the modal view around the palette's [`Picker`]. It carries the `Palette`
//! key context (the picker's own `Picker` / `Picker > Input` contexts sit inside it), so the
//! palette's bindings (`palette::ToggleShowAll`) live in `Palette` and the picker's navigation in
//! `Picker`; printable keys reach the query field.

use gpui::{
    App, AppContext as _, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, Subscription, Window, div,
};
use oxikube_keymap::{KeyContextual, contexts};
use oxikube_workspace::ModalView;

use super::ToggleShowAll;
use super::delegate::{CommandPaletteDelegate, PaletteParts};
use crate::picker::Picker;

/// The command palette as a modal. Open it with
/// [`PaletteHost::toggle`](super::PaletteHost::toggle).
pub struct CommandPalette {
    picker: Entity<Picker<CommandPaletteDelegate>>,
    _dismiss: Subscription,
}

impl CommandPalette {
    /// A palette over `parts`.
    pub fn new(parts: PaletteParts, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let picker =
            cx.new(|cx| Picker::uniform_list(CommandPaletteDelegate::new(parts), window, cx));
        // The picker closes itself (confirm, Escape); the modal layer listens to this view.
        let dismiss = cx.subscribe(&picker, |_, _, _: &DismissEvent, cx| cx.emit(DismissEvent));
        Self {
            picker,
            _dismiss: dismiss,
        }
    }

    /// The picker, for tests and the host.
    pub fn picker(&self) -> &Entity<Picker<CommandPaletteDelegate>> {
        &self.picker
    }

    /// Lists the unavailable commands too, or hides them again (`palette::ToggleShowAll`).
    pub fn toggle_show_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.picker.update(cx, |picker, cx| {
            picker.delegate.show_all = !picker.delegate.show_all;
            picker.refresh(window, cx);
            cx.notify();
        });
    }
}

impl EventEmitter<DismissEvent> for CommandPalette {}

impl Focusable for CommandPalette {
    /// The query field: the palette opens with the caret there.
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.picker.focus_handle(cx)
    }
}

impl KeyContextual for CommandPalette {
    const KEY_CONTEXT: &'static str = contexts::PALETTE;
}

impl ModalView for CommandPalette {
    fn on_before_dismiss(&mut self, window: &mut Window, cx: &mut App) -> bool {
        self.picker
            .update(cx, |picker, cx| picker.on_before_dismiss(window, cx))
    }
}

impl Render for CommandPalette {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("command-palette")
            .key_context(self.key_context())
            .on_action(cx.listener(|this, _: &ToggleShowAll, window, cx| {
                this.toggle_show_all(window, cx);
            }))
            .child(self.picker.clone())
    }
}
