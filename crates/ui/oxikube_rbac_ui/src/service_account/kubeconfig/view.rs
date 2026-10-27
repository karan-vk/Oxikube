//! Drawing the kubeconfig section: the choices, the button and the result.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, div, px,
};
use oxikube_app::sa_kubeconfig::GeneratedKubeconfig;
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::checkbox::Checkbox;
use oxikube_ui::layout::{Disableable as _, Selectable as _, StyledExt as _, h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Sizable as _, Tokens, u};

use super::state::Expiry;
use crate::service_account::panel::ServiceAccountPanel;

/// The preview shows at most this many lines (the file is short; this guards a long CA).
const MAX_PREVIEW_LINES: usize = 40;

impl ServiceAccountPanel {
    /// The "Kubeconfig" section: lifetime, the button and, once generated, the preview.
    pub(in crate::service_account) fn kubeconfig_section(
        &self,
        tokens: &Tokens,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let state = &self.kubeconfig;
        v_flex()
            .id("sa-kubeconfig")
            .debug_selector(|| "sa-kubeconfig".to_owned())
            .px(u(tokens.spacing.lg))
            .py(u(tokens.spacing.sm))
            .gap(u(tokens.spacing.sm))
            .child(
                div()
                    .font_semibold()
                    .child("Kubeconfig"),
            )
            .child(self.lifetime_row(tokens, cx))
            .child(
                div()
                    .debug_selector(|| "sa-kubeconfig-legacy".to_owned())
                    .text_size(u(tokens.font.small))
                    .child(
                        Checkbox::new("sa-kubeconfig-legacy-box")
                            .label("If the cluster has no TokenRequest, allow a long-lived token Secret")
                            .checked(state.allow_legacy)
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.set_kubeconfig_allow_legacy(*checked, cx);
                            })),
                    ),
            )
            .child(
                h_flex()
                    .gap(u(tokens.spacing.md))
                    .items_center()
                    .child(
                        div()
                            .debug_selector(|| "sa-kubeconfig-generate".to_owned())
                            .child(
                                Button::new("sa-kubeconfig-generate-button")
                                    .xsmall()
                                    .label("Generate kubeconfig")
                                    .disabled(state.waiting)
                                    .tooltip(
                                        "Requests a token for this account and builds a kubeconfig (asks to confirm)",
                                    )
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.generate_kubeconfig(cx);
                                    })),
                            ),
                    )
                    .when(state.waiting, |row| {
                        row.child(
                            div()
                                .debug_selector(|| "sa-kubeconfig-waiting".to_owned())
                                .text_color(tokens.colors.text_muted)
                                .child("Requesting the token…"),
                        )
                    }),
            )
            .when_some(state.generated.as_ref(), |column, generated| {
                column.child(self.result(tokens, generated, cx))
            })
            .when_some(state.note.clone(), |column, note| {
                column.child(
                    div()
                        .debug_selector(|| "sa-kubeconfig-note".to_owned())
                        .text_color(tokens.colors.text_muted)
                        .child(note),
                )
            })
            .into_any_element()
    }

    fn lifetime_row(&self, tokens: &Tokens, cx: &mut Context<Self>) -> AnyElement {
        let current = self.kubeconfig.expiry;
        h_flex()
            .gap(u(tokens.spacing.sm))
            .items_center()
            .child(
                div()
                    .text_color(tokens.colors.text_muted)
                    .child("Valid for"),
            )
            .children(Expiry::ALL.into_iter().map(|expiry| {
                div()
                    .debug_selector(move || format!("sa-kubeconfig-expiry-{}", expiry.seconds()))
                    .child(
                        Button::new(("sa-kubeconfig-expiry", expiry.seconds() as usize))
                            .xsmall()
                            .ghost()
                            .label(expiry.label())
                            .selected(expiry == current)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.set_kubeconfig_expiry(expiry, cx);
                            })),
                    )
            }))
            .into_any_element()
    }

    /// The finished kubeconfig: what the token is good for, a warning if it never expires, the
    /// text with the token masked, and the buttons that take it out of the app.
    fn result(
        &self,
        tokens: &Tokens,
        generated: &GeneratedKubeconfig,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let reveal = self.kubeconfig.reveal;
        let lifetime = match &generated.expires_at {
            Some(when) => format!("The token expires at {when}."),
            None => "The token does not expire.".to_owned(),
        };
        let text = if reveal {
            generated.text.expose()
        } else {
            generated.preview.as_str()
        };
        let lines: Vec<SharedString> = text
            .lines()
            .take(MAX_PREVIEW_LINES)
            .map(|line| SharedString::from(line.to_owned()))
            .collect();
        v_flex()
            .gap(u(tokens.spacing.sm))
            .child(
                div()
                    .debug_selector(|| "sa-kubeconfig-expiry-note".to_owned())
                    .text_color(tokens.colors.text_muted)
                    .child(lifetime),
            )
            .when_some(generated.warning.clone(), |column, warning| {
                column.child(
                    div()
                        .debug_selector(|| "sa-kubeconfig-warning".to_owned())
                        .text_color(tokens.colors.warning)
                        .child(warning),
                )
            })
            .child(
                h_flex()
                    .gap(u(tokens.spacing.sm))
                    .child(
                        div()
                            .debug_selector(|| "sa-kubeconfig-copy".to_owned())
                            .child(
                                Button::new("sa-kubeconfig-copy-button")
                                    .xsmall()
                                    .label("Copy")
                                    .tooltip(
                                        "Copy the kubeconfig, token included, to the clipboard",
                                    )
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.copy_kubeconfig(cx);
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .debug_selector(|| "sa-kubeconfig-save".to_owned())
                            .child(
                                Button::new("sa-kubeconfig-save-button")
                                    .xsmall()
                                    .label("Save…")
                                    .tooltip("Save the kubeconfig to a file only you can read")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.save_kubeconfig(cx);
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .debug_selector(|| "sa-kubeconfig-reveal".to_owned())
                            .child(
                                Button::new("sa-kubeconfig-reveal-button")
                                    .xsmall()
                                    .ghost()
                                    .label(if reveal { "Hide token" } else { "Show token" })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.toggle_kubeconfig_reveal(cx);
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .debug_selector(|| "sa-kubeconfig-discard".to_owned())
                            .child(
                                Button::new("sa-kubeconfig-discard-button")
                                    .xsmall()
                                    .ghost()
                                    .label("Discard")
                                    .tooltip("Forget the token held in memory")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.discard_kubeconfig(cx);
                                    })),
                            ),
                    ),
            )
            .child(
                v_flex()
                    .id("sa-kubeconfig-preview")
                    .debug_selector(|| "sa-kubeconfig-preview".to_owned())
                    .max_h(px(220.))
                    .overflow_y_scroll()
                    .p(u(tokens.spacing.sm))
                    .rounded(u(tokens.radius.sm))
                    .bg(tokens.colors.surface)
                    .border_1()
                    .border_color(tokens.colors.border_variant)
                    .font_family(cx.mono_font_family())
                    .text_size(u(tokens.font.small))
                    .children(lines.into_iter().map(|line| div().child(line))),
            )
            .into_any_element()
    }
}
