//! The body of each state.

use gpui::{
    Context, InteractiveElement as _, IntoElement as _, ParentElement as _, Styled as _, Window,
    div, prelude::FluentBuilder as _,
};
use oxikube_ui::spinner::Spinner;
use oxikube_ui::{ActiveTokens as _, Icon, IconName, Sizable as _, u};

use super::ConnectView;
use super::parts::{actions, button, card, details_box, link, note};
use crate::connect::model::{
    AuthRequiredModel, ConnectViewModel, ConnectingModel, DisconnectedModel, ErrorModel,
    TerminalAction,
};

impl ConnectView {
    /// The view's frame: the body of its state. Nothing for the states in which the cluster tab
    /// shows the cluster's own content (the tab does not draw this view then).
    pub(super) fn render_body(
        &mut self,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        match self.model.clone() {
            ConnectViewModel::Content | ConnectViewModel::Degraded(_) => {
                div().size_full().into_any_element()
            }
            ConnectViewModel::Disconnected(model) => self.render_disconnected(&model, cx),
            ConnectViewModel::Connecting(model) => self.render_connecting(&model, cx),
            ConnectViewModel::AuthRequired(model) => self.render_auth(&model, cx),
            ConnectViewModel::Error(model) => self.render_error(&model, cx),
        }
    }

    /// The "Show details" and "Copy details" row. `toggle`: the text is longer than what the
    /// body shows; `copy`: there is text worth copying.
    fn details_controls(&self, toggle: bool, copy: bool, cx: &mut Context<Self>) -> gpui::Div {
        let toggle_view = cx.entity();
        let copy_view = cx.entity();
        let open = self.details_open;
        actions()
            .when(toggle, |row| {
                row.child(link(
                    "connect-details-toggle",
                    if open { "Hide details" } else { "Show details" },
                    move |_, cx| toggle_view.update(cx, |this, cx| this.toggle_details(cx)),
                ))
            })
            .when(copy, |row| {
                row.child(link("connect-copy", "Copy details", move |_, cx| {
                    copy_view.update(cx, |this, cx| this.copy_details(cx))
                }))
            })
    }

    fn render_disconnected(
        &mut self,
        model: &DisconnectedModel,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let colors = cx.colors();
        let this = cx.entity();
        card(
            "connect-disconnected",
            IconName::Plug,
            colors.text_disabled,
            format!("{} is not connected", model.title),
            cx,
        )
        .child(
            actions().child(button("connect-connect", "Connect", true, true, {
                move |_, cx| this.update(cx, |this, cx| this.connect(cx))
            })),
        )
        .into_any_element()
    }

    fn render_connecting(
        &mut self,
        model: &ConnectingModel,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let tokens = cx.tokens();
        let this = cx.entity();
        let server = model
            .server
            .clone()
            .unwrap_or_else(|| "API server address not known".to_owned());
        card(
            "connect-connecting",
            IconName::Server,
            tokens.colors.text_disabled,
            format!("Connecting to {}", model.title),
            cx,
        )
        .child(
            div().debug_selector(|| "connect-spinner".to_owned()).child(
                Spinner::new()
                    .icon(Icon::new(IconName::LoaderCircle))
                    .large()
                    .color(tokens.colors.accent),
            ),
        )
        .child(note("connect-server", server, cx))
        .child(note(
            "connect-context",
            format!("context {}", model.context),
            cx,
        ))
        .child(
            actions().child(button("connect-cancel", "Cancel", false, true, {
                move |_, cx| this.update(cx, |this, cx| this.cancel(cx))
            })),
        )
        .into_any_element()
    }

    fn render_auth(
        &mut self,
        model: &AuthRequiredModel,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let retry = cx.entity();
        let terminal = cx.entity();
        let message = if model.message.is_empty() {
            "The cluster asked for credentials.".to_owned()
        } else {
            model.message.summary.clone()
        };
        let policy = format!(
            "Interactive sign-in: {} ({}). {}",
            model.policy.label(),
            model.policy.setting_value(),
            model.policy.explanation()
        );
        let terminal_ready = model.terminal == TerminalAction::Available;
        card(
            "connect-auth",
            IconName::KeyRound,
            colors.warning,
            format!("Sign-in required for {}", model.title),
            cx,
        )
        .child(note("connect-auth-message", message, cx).text_color(colors.text))
        .child(self.details_controls(model.message.truncated, model.message.truncated, cx))
        .when(self.details_open && model.message.truncated, |card| {
            card.child(details_box("connect-details-box", &model.message.full, cx))
        })
        .child(note("connect-auth-instructions", model.instructions, cx))
        .child(note("connect-policy", policy, cx).text_size(u(tokens.font.small)))
        .child(
            actions()
                .child(button(
                    "connect-open-terminal",
                    "Open terminal",
                    false,
                    terminal_ready,
                    move |_, cx| terminal.update(cx, |this, cx| this.open_terminal(cx)),
                ))
                .child(button(
                    "connect-retry",
                    "Retry",
                    true,
                    true,
                    move |_, cx| retry.update(cx, |this, cx| this.retry(cx)),
                )),
        )
        .when(!terminal_ready, |card| {
            card.child(
                note(
                    "connect-terminal-unavailable",
                    "A terminal for signing in is not available yet.",
                    cx,
                )
                .text_size(u(tokens.font.small))
                .text_color(colors.text_disabled),
            )
        })
        .into_any_element()
    }

    fn render_error(&mut self, model: &ErrorModel, cx: &mut Context<Self>) -> gpui::AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let retry = cx.entity();
        let sources = cx.entity();
        let summary = if model.message.is_empty() {
            "The connection failed.".to_owned()
        } else {
            model.message.summary.clone()
        };
        let open = self.details_open;
        card(
            "connect-error",
            IconName::CircleAlert,
            colors.error,
            format!("Could not connect to {}", model.title),
            cx,
        )
        .child(note("connect-error-summary", summary, cx).text_color(colors.text))
        .child(self.details_controls(true, true, cx))
        .when(open, |card| {
            card.child(details_box("connect-details-box", &model.details, cx))
        })
        .child(
            actions()
                .when(model.sources, |row| {
                    row.child(link(
                        "connect-edit-sources",
                        "Edit kubeconfig sources",
                        move |_, cx| sources.update(cx, |this, cx| this.open_sources(cx)),
                    ))
                })
                .child(button(
                    "connect-retry",
                    "Retry",
                    true,
                    true,
                    move |_, cx| retry.update(cx, |this, cx| this.retry(cx)),
                )),
        )
        .into_any_element()
    }
}
