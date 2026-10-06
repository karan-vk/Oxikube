//! [`DialogModal`]: a ready-made confirmation dialog for the modal layer.
//!
//! Built from `oxikube_ui`'s dialog pieces (header, title, description, footer) and buttons, so it
//! looks like the rest of the app's dialogs. Mutation confirmations (E06-S02) and "are you sure"
//! prompts open one through `Workspace::toggle_modal`. Enter confirms, Escape cancels; the
//! dialog itself takes focus when it opens and Tab walks its two buttons.

use gpui::{
    App, Context, DismissEvent, EventEmitter, FocusHandle, Focusable, InteractiveElement as _,
    IntoElement, KeyBinding, NoAction, ParentElement as _, Render, SharedString, Styled as _,
    Window, div, px,
};
use oxikube_ui::{
    ActiveTokens as _,
    button::{Button, ButtonVariants as _},
    dialog::{Cancel, Confirm, DialogDescription, DialogFooter, DialogHeader, DialogTitle},
    layout::v_flex,
    u,
};

use super::{ModalPlacement, ModalView};

/// The key context of the dialog.
pub const DIALOG_KEY_CONTEXT: &str = "DialogModal";

/// The key context wrapped around each footer button, so Enter reaches the focused button.
const BUTTON_KEY_CONTEXT: &str = "DialogButton";

/// Registers Enter = confirm in the dialog's context. Called by [`super::register`].
///
/// GPUI matches key bindings before it hands a key-down to the focused element, so a bare Enter
/// binding on the dialog would confirm even while Tab has put focus on Cancel. The footer buttons
/// therefore sit in a [`BUTTON_KEY_CONTEXT`] where Enter is unbound, and the button's own
/// Enter-to-click handling runs. Enter with the dialog itself focused still confirms.
pub(super) fn register(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new(
            "enter",
            Confirm { secondary: false },
            Some(DIALOG_KEY_CONTEXT),
        ),
        KeyBinding::new("enter", NoAction, Some(BUTTON_KEY_CONTEXT)),
    ]);
}

type Handler = Box<dyn Fn(&mut Window, &mut App)>;

/// A title, an optional message and a confirm / cancel pair of buttons.
pub struct DialogModal {
    title: SharedString,
    message: Option<SharedString>,
    confirm_label: SharedString,
    cancel_label: SharedString,
    destructive: bool,
    on_confirm: Option<Handler>,
    on_cancel: Option<Handler>,
    focus_handle: FocusHandle,
}

impl DialogModal {
    /// A dialog titled `title` with "OK" and "Cancel" buttons.
    pub fn new(title: impl Into<SharedString>, cx: &mut Context<Self>) -> Self {
        Self {
            title: title.into(),
            message: None,
            confirm_label: "OK".into(),
            cancel_label: "Cancel".into(),
            destructive: false,
            on_confirm: None,
            on_cancel: None,
            focus_handle: cx.focus_handle(),
        }
    }

    /// The explanatory text under the title.
    pub fn message(mut self, message: impl Into<SharedString>) -> Self {
        self.message = Some(message.into());
        self
    }

    /// The confirm button's label.
    pub fn confirm_label(mut self, label: impl Into<SharedString>) -> Self {
        self.confirm_label = label.into();
        self
    }

    /// The cancel button's label.
    pub fn cancel_label(mut self, label: impl Into<SharedString>) -> Self {
        self.cancel_label = label.into();
        self
    }

    /// Draws the confirm button as a dangerous action.
    pub fn destructive(mut self) -> Self {
        self.destructive = true;
        self
    }

    /// Runs when the user confirms; the dialog then closes.
    pub fn on_confirm(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_confirm = Some(Box::new(handler));
        self
    }

    /// Runs when the user cancels (button or Escape); the dialog then closes.
    pub fn on_cancel(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_cancel = Some(Box::new(handler));
        self
    }

    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(handler) = &self.on_confirm {
            handler(window, cx);
        }
        cx.emit(DismissEvent);
    }

    fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(handler) = &self.on_cancel {
            handler(window, cx);
        }
        cx.emit(DismissEvent);
    }
}

impl EventEmitter<DismissEvent> for DialogModal {}

impl Focusable for DialogModal {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl ModalView for DialogModal {
    fn placement(&self, _: &App) -> ModalPlacement {
        ModalPlacement::Center
    }
}

impl Render for DialogModal {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let mut confirm = Button::new("dialog-confirm").label(self.confirm_label.clone());
        confirm = if self.destructive {
            confirm.danger()
        } else {
            confirm.primary()
        };
        let confirm = confirm.on_click(cx.listener(|this, _, window, cx| this.confirm(window, cx)));
        let cancel = Button::new("dialog-cancel")
            .label(self.cancel_label.clone())
            .on_click(cx.listener(|this, _, window, cx| this.cancel(window, cx)));

        v_flex()
            .id("dialog-modal")
            .debug_selector(|| "dialog-modal".to_owned())
            .key_context(DIALOG_KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .tab_group()
            .w(u(px(420.)))
            .gap(u(tokens.spacing.xl))
            .p(u(tokens.spacing.xxl))
            .bg(colors.elevated_surface)
            .text_color(colors.text)
            .border_1()
            .border_color(colors.border)
            .rounded(u(tokens.radius.lg))
            .shadow_lg()
            .on_action(cx.listener(|this, _: &Cancel, window, cx| this.cancel(window, cx)))
            .on_action(cx.listener(|this, _: &Confirm, window, cx| this.confirm(window, cx)))
            .child(
                DialogHeader::new()
                    .child(DialogTitle::new().child(self.title.clone()))
                    .children(
                        self.message
                            .clone()
                            .map(|message| DialogDescription::new().child(message)),
                    ),
            )
            .child(
                DialogFooter::new()
                    .child(
                        div()
                            .key_context(BUTTON_KEY_CONTEXT)
                            .debug_selector(|| "dialog-cancel".to_owned())
                            .child(cancel),
                    )
                    .child(
                        div()
                            .key_context(BUTTON_KEY_CONTEXT)
                            .debug_selector(|| "dialog-confirm".to_owned())
                            .child(confirm),
                    ),
            )
    }
}
