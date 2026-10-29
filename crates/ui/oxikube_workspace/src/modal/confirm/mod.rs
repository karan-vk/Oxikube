//! [`ConfirmDialog`]: the one confirmation dialog of the guard (E19-S06).
//!
//! The guard never waits for the UI: a mutation that needs confirming comes back as a
//! `ConfirmationRequest`, and this dialog renders it (as a [`ConfirmPrompt`]) and answers by
//! calling the closures its opener gave it: confirm (re-dispatch with the answer) or decline
//! (the guard audits the refusal).
//!
//! - **Simple** tier: Confirm is enabled; the dialog holds focus, so Enter confirms and Esc
//!   cancels; Tab walks Cancel and Confirm.
//! - **Type-the-name** tier: the field takes focus; Confirm stays disabled and Enter does nothing
//!   until the field holds the expected text exactly ([`typed_matches`]: case-sensitive, no
//!   trimming, as the guard compares). The check is one string compare per keystroke.
//! - **Bulk**: the objects are listed in a virtualised list, with their own risk when they differ.
//!
//! Every way the dialog goes away answers it: Confirm and Cancel directly, and Esc, a click on the
//! scrim, a replacement modal or a closing window by releasing the entity, which declines (the
//! flag-and-release pattern, no task involved). A request is therefore never left pending.

mod prompt;
mod render;
pub(crate) mod request;

use gpui::{
    App, AppContext as _, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable,
    SharedString, Subscription, Window,
};
use oxikube_ui::input::{InputEvent, InputState};

pub use prompt::{ConfirmItem, ConfirmPrompt, PreviewNote, typed_matches};

use super::{ModalPlacement, ModalView};

/// Runs when the user confirms, with what they typed (`None` for a simple confirmation).
type ConfirmHandler = Box<dyn FnOnce(Option<String>, &mut Window, &mut App)>;

/// Runs when the dialog goes away without a confirmation.
type DeclineHandler = Box<dyn FnOnce(&mut App)>;

/// The text field of a type-the-name prompt.
struct TypedField {
    expected: SharedString,
    input: Entity<InputState>,
    _subscription: Subscription,
}

/// The guard's confirmation dialog. See the module docs.
pub struct ConfirmDialog {
    prompt: ConfirmPrompt,
    typed: Option<TypedField>,
    on_confirm: Option<ConfirmHandler>,
    on_decline: Option<DeclineHandler>,
    focus_handle: FocusHandle,
}

impl ConfirmDialog {
    /// A dialog for `prompt`. `on_confirm` runs once on a valid confirmation; `on_decline` runs
    /// once if the dialog ends any other way. Exactly one of them runs.
    pub fn new(
        prompt: ConfirmPrompt,
        on_confirm: impl FnOnce(Option<String>, &mut Window, &mut App) + 'static,
        on_decline: impl FnOnce(&mut App) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let typed = prompt.expected.clone().map(|expected| {
            let input =
                cx.new(|cx| InputState::new(window, cx).placeholder("Type the name to confirm"));
            let subscription =
                cx.subscribe_in(&input, window, |this, _, event, window, cx| match event {
                    InputEvent::Change => cx.notify(),
                    InputEvent::PressEnter { .. } => this.press_confirm(window, cx),
                    InputEvent::Focus | InputEvent::Blur => {}
                });
            TypedField {
                expected,
                input,
                _subscription: subscription,
            }
        });
        // Whatever ends the dialog without an answer (Esc through the layer, the scrim, another
        // modal opening, the window closing) releases the entity: decline then.
        cx.on_release(|this, cx| {
            if let Some(decline) = this.on_decline.take() {
                decline(cx);
            }
        })
        .detach();
        Self {
            prompt,
            typed,
            on_confirm: Some(Box::new(on_confirm)),
            on_decline: Some(Box::new(on_decline)),
            focus_handle: cx.focus_handle(),
        }
    }

    /// Puts focus where the tier wants it: in the field for type-the-name, on the dialog (whose
    /// Enter confirms) otherwise. Call after the modal layer has shown the dialog, which focuses
    /// the dialog itself.
    pub fn focus_initial(&self, window: &mut Window, cx: &mut Context<Self>) {
        match &self.typed {
            Some(typed) => typed.input.update(cx, |input, cx| input.focus(window, cx)),
            None => window.focus(&self.focus_handle, cx),
        }
    }

    /// The dialog's title.
    pub fn title(&self) -> &SharedString {
        &self.prompt.title
    }

    /// The one-line summary under the title.
    pub fn message_text(&self) -> &SharedString {
        &self.prompt.summary
    }

    /// The confirm button's label.
    pub fn confirm_text(&self) -> &SharedString {
        &self.prompt.confirm_label
    }

    /// The text the user has to type, when the dialog asks for one.
    pub fn expected_text(&self) -> Option<&SharedString> {
        self.typed.as_ref().map(|typed| &typed.expected)
    }

    /// Types `text` into the confirmation field, as the user would.
    pub fn type_text(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(typed) = &self.typed {
            typed
                .input
                .update(cx, |input, cx| input.set_value(text.to_owned(), window, cx));
        }
    }

    /// Whether Confirm can be pressed: always, unless a name has to be typed first and the field
    /// does not hold it exactly.
    pub fn can_confirm(&self, cx: &App) -> bool {
        self.typed.as_ref().is_none_or(|typed| {
            typed_matches(typed.input.read(cx).value().as_ref(), &typed.expected)
        })
    }

    /// Presses Confirm (a click, Enter). Does nothing while the typed name is wrong.
    pub fn press_confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_confirm(cx) {
            return;
        }
        let typed = self
            .typed
            .as_ref()
            .map(|typed| typed.input.read(cx).value().to_string());
        // Taking the decline handler marks the dialog answered, so the release does not decline.
        self.on_decline = None;
        if let Some(confirm) = self.on_confirm.take() {
            confirm(typed, window, cx);
        }
        cx.emit(DismissEvent);
    }

    /// Presses Cancel (a click, Esc): declines and closes.
    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        self.on_confirm = None;
        if let Some(decline) = self.on_decline.take() {
            decline(cx);
        }
        cx.emit(DismissEvent);
    }
}

impl EventEmitter<DismissEvent> for ConfirmDialog {}

impl Focusable for ConfirmDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl ModalView for ConfirmDialog {
    /// The user dismissed the dialog (Esc through the layer, a click on the scrim): that is a
    /// "no", answered now rather than whenever the entity is released.
    fn on_before_dismiss(&mut self, _: &mut Window, cx: &mut App) -> bool {
        self.on_confirm = None;
        if let Some(decline) = self.on_decline.take() {
            decline(cx);
        }
        true
    }

    fn placement(&self, _: &App) -> ModalPlacement {
        ModalPlacement::Center
    }
}
