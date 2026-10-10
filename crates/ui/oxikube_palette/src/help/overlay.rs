//! [`HelpOverlay`]: the modal the workspace's modal layer shows.
//!
//! A [`Picker`] over a [`HelpDelegate`], wrapped in a dialog: the wrapper names it for assistive
//! technology (role `Dialog`, label "Keyboard shortcuts"), sets the `Help` key context, and
//! forwards the picker's dismissal to the modal layer, which closes the overlay and hands the
//! focus back to the view that had it.
//!
//! The `Help` context carries `empty` while the search field is empty, so the keymap can bind `?`
//! to close the overlay only then (`"Help && empty > Input"`): with text in the field `?` is a
//! character to search for.

use std::sync::Arc;

use gpui::{
    App, AppContext as _, Context, DismissEvent, Div, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, KeyContext, ParentElement as _, Render, Role, Stateful,
    StatefulInteractiveElement as _, Subscription, Window, px,
};
use oxikube_keymap::{KeyContextBuilder, contexts};
use oxikube_workspace::modal::ModalView;

use super::delegate::HelpDelegate;
use super::model::HelpModel;
use crate::picker::Picker;

/// The overlay's width (before UI zoom): the title, the action name, a chip and the keys.
const WIDTH: f32 = 680.;
/// The most the list grows to before it scrolls (before UI zoom).
const MAX_HEIGHT: f32 = 460.;

/// The accessible name of the overlay.
pub const ACCESSIBLE_NAME: &str = "Keyboard shortcuts";

/// The help overlay. See the [module docs](self).
pub struct HelpOverlay {
    picker: Entity<Picker<HelpDelegate>>,
    _dismiss: Subscription,
    _query: Subscription,
}

impl HelpOverlay {
    /// An overlay listing `model`'s bindings, its search field focused when the modal layer
    /// shows it.
    pub fn new(model: Arc<HelpModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let picker = cx.new(|cx| {
            Picker::uniform_list(HelpDelegate::new(model), window, cx)
                .width(px(WIDTH))
                .max_height(px(MAX_HEIGHT))
        });
        let dismiss = cx.subscribe(&picker, |_, _, _: &DismissEvent, cx| cx.emit(DismissEvent));
        // The `empty` flag follows the query: re-render when the picker's matches change.
        let query = cx.observe(&picker, |_, _, cx| cx.notify());
        Self {
            picker,
            _dismiss: dismiss,
            _query: query,
        }
    }

    /// The picker inside, for tests.
    pub fn picker(&self) -> &Entity<Picker<HelpDelegate>> {
        &self.picker
    }

    fn key_context(&self, cx: &App) -> KeyContext {
        let mut builder = KeyContextBuilder::new(contexts::HELP);
        builder.flag_if(self.picker.read(cx).query(cx).is_empty(), "empty");
        builder.build()
    }
}

impl EventEmitter<DismissEvent> for HelpOverlay {}

impl Focusable for HelpOverlay {
    /// The search field: opening the overlay puts the caret there.
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.picker.read(cx).focus_handle(cx)
    }
}

impl ModalView for HelpOverlay {}

impl Render for HelpOverlay {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        frame(self.key_context(cx)).child(self.picker.clone())
    }
}

/// The dialog box around the picker: its id, role (`Dialog`), accessible name and key context.
pub(super) fn frame(context: KeyContext) -> Stateful<Div> {
    gpui::div()
        .id("help-overlay")
        .debug_selector(|| "help-overlay".to_owned())
        .role(Role::Dialog)
        .aria_label(ACCESSIBLE_NAME)
        .key_context(context)
}
