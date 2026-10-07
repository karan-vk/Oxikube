//! The strip under the toolbar once the stream stopped: what happened, said once, and the way on.
//!
//! A failure is one plain sentence ([`HumanError::summary`], no `internal error:` label, no raw
//! address) with a "Details" toggle that opens the adapter's own text. The window draws no state
//! row for the failures this strip covers (see [`LineWindow::shows_state`]), so nothing says it
//! twice. The button is Reconnect, Follow replacement for a replaced pod, or Close when the pod
//! or container no longer exists and reading it again cannot succeed.

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement as _, ParentElement as _,
    Styled as _, div, prelude::FluentBuilder as _,
};
use oxikube_app::logs::{LogFailure, LogState};
use oxikube_domain::HumanError;
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::error_details::{self, details_box, details_toggle};
use oxikube_ui::layout::{h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Sizable as _, u};
use oxikube_workspace::ItemEvent;

use super::text::state_text;
use super::{LogView, Recovery};

/// A failure of the log stream as the user reads it.
fn human_failure(failure: &LogFailure, what: &str) -> HumanError {
    let error = HumanError::new(failure.kind, &failure.message);
    if error.is_not_found() {
        let what = what.to_lowercase();
        return error.with_summary(format!(
            "This {what} was not found. It may have been deleted."
        ));
    }
    error
}

impl LogView {
    /// What the strip says now: the sentence, and the raw text behind the Details toggle.
    pub fn recovery_error(&self) -> Option<HumanError> {
        match self.window.state() {
            LogState::Failed(failure) => Some(human_failure(failure, &self.target.gvk.kind)),
            _ => None,
        }
    }

    /// Whether the strip shows the raw text of the failure.
    pub fn error_details_open(&self) -> bool {
        self.error_details_open
    }

    /// Opens or closes the strip's raw text.
    pub fn toggle_error_details(&mut self, cx: &mut Context<Self>) {
        self.error_details_open = !self.error_details_open;
        cx.notify();
    }

    /// Closes this tab (the strip's Close): the pod is gone, there is nothing left to read.
    pub fn close_tab(&mut self, cx: &mut Context<Self>) {
        cx.emit(ItemEvent::CloseItem);
    }

    /// The strip that offers [`LogView::recovery`]: what happened, and the button.
    pub(crate) fn recovery_strip(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let recovery = self.recovery()?;
        let error = self.recovery_error();
        let words = match (&error, recovery) {
            (Some(error), _) => error.summary().to_owned(),
            (None, Recovery::FollowReplacement) => {
                "The pod was replaced. Its replacement's log is one click away.".to_owned()
            }
            (None, _) => state_text(self.window.state()),
        };
        let (id, label) = match recovery {
            Recovery::FollowReplacement => ("log-follow-replacement", "Follow replacement"),
            Recovery::Reconnect => ("log-reconnect", "Reconnect"),
            Recovery::Close => ("log-close", "Close"),
        };
        let tokens = cx.tokens();
        let button = Button::new(id)
            .label(label)
            .primary()
            .xsmall()
            .on_click(cx.listener(move |view, _, _, cx| match recovery {
                Recovery::FollowReplacement => view.request_follow_replacement(cx),
                Recovery::Reconnect => view.request_reconnect(cx),
                Recovery::Close => view.close_tab(cx),
            }));
        let details = error.filter(HumanError::has_details);
        let open = self.error_details_open;
        let toggle = cx.entity();
        Some(
            v_flex()
                .id("log-recovery")
                .debug_selector(|| "log-recovery".into())
                .flex_none()
                .gap(u(tokens.spacing.sm))
                .px(u(tokens.spacing.md))
                .py(u(tokens.spacing.sm))
                .bg(tokens.colors.surface)
                .border_b_1()
                .border_color(tokens.colors.border_variant)
                .child(
                    h_flex()
                        .items_center()
                        .gap(u(tokens.spacing.md))
                        .child(
                            div()
                                .debug_selector(|| "log-recovery-summary".into())
                                .flex_1()
                                .min_w_0()
                                .text_size(u(tokens.font.small))
                                .child(words),
                        )
                        .when(details.is_some(), |row| {
                            row.child(details_toggle("log-details-toggle", open, move |_, cx| {
                                toggle.update(cx, |view, cx| view.toggle_error_details(cx));
                            }))
                        })
                        .child(div().debug_selector(move || id.into()).child(button)),
                )
                .when_some(details.filter(|_| open), |strip, error| {
                    strip.child(details_box(
                        "log-details-box",
                        "log-details",
                        error.raw(),
                        error_details::SHORT,
                        cx,
                    ))
                })
                .into_any_element(),
        )
    }
}
