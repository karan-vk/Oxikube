//! The detail pane's Values, Manifest and Notes tabs: code views and the notes text.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    Styled as _, div, px,
};
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::layout::{h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Icon, IconName, Sizable as _, u};

use super::detail::ReleaseDetail;
use super::model::ValuesMode;
use super::view::HelmReleasesView;

impl HelmReleasesView {
    pub(super) fn reveal_button(&self, revealed: bool, cx: &mut Context<Self>) -> AnyElement {
        div()
            .debug_selector(|| "helm-release-reveal".into())
            .child(
                Button::new("helm-release-reveal-button")
                    .ghost()
                    .xsmall()
                    .icon(
                        Icon::new(if revealed {
                            IconName::EyeOff
                        } else {
                            IconName::Eye
                        })
                        .size(u(px(12.))),
                    )
                    .label(if revealed {
                        "Hide values"
                    } else {
                        "Reveal values"
                    })
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_reveal(cx))),
            )
            .into_any_element()
    }

    pub(super) fn values_body(&self, detail: &ReleaseDetail, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let facts = detail.values_facts;
        let mode = detail.mode;
        let mode_button = |label: &'static str, this_mode: ValuesMode, cx: &mut Context<Self>| {
            let button = Button::new(("helm-release-mode", this_mode as usize))
                .xsmall()
                .label(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    if this.detail.as_ref().is_some_and(|d| d.mode != this_mode) {
                        this.toggle_values_mode(cx);
                    }
                }));
            div()
                .debug_selector(move || format!("helm-release-mode:{this_mode:?}"))
                .child(if mode == this_mode {
                    button.primary()
                } else {
                    button.ghost()
                })
        };
        let masking: Option<SharedString> = (facts.sensitive > 0).then(|| {
            if facts.revealed {
                format!("{} sensitive values shown", facts.sensitive).into()
            } else {
                format!("{} sensitive values masked", facts.sensitive).into()
            }
        });
        let body = if detail.values.empty {
            self.note(
                "helm-release-values-empty",
                match mode {
                    ValuesMode::User => {
                        "No user-supplied values: this release runs on the chart's defaults."
                    }
                    ValuesMode::All => "This release has no values.",
                },
                false,
                cx,
            )
        } else {
            div()
                .debug_selector(|| "helm-release-values".into())
                .size_full()
                .child(detail.values.code.clone())
                .into_any_element()
        };
        v_flex()
            .size_full()
            .child(
                h_flex()
                    .flex_none()
                    .gap(u(tokens.spacing.sm))
                    .px(u(tokens.spacing.xl))
                    .py(u(tokens.spacing.sm))
                    .items_center()
                    .child(mode_button("User", ValuesMode::User, cx))
                    .child(mode_button("All", ValuesMode::All, cx))
                    .children(masking.map(|text| {
                        div()
                            .debug_selector(|| "helm-release-masking".into())
                            .ml(u(tokens.spacing.md))
                            .text_size(u(tokens.font.small))
                            .text_color(tokens.colors.text_muted)
                            .child(text)
                    }))
                    .child(div().flex_1())
                    .children(
                        (facts.sensitive > 0 || facts.revealed)
                            .then(|| self.reveal_button(facts.revealed, cx)),
                    ),
            )
            .child(div().flex_1().min_h_0().child(body))
            .into_any_element()
    }

    pub(super) fn manifest_body(
        &self,
        detail: &ReleaseDetail,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tokens = cx.tokens();
        if detail.manifest.empty {
            return self.note(
                "helm-release-manifest-empty",
                "This revision has no manifest.",
                false,
                cx,
            );
        }
        let masked = detail.manifest_masked;
        v_flex()
            .size_full()
            .child(
                h_flex()
                    .flex_none()
                    .gap(u(tokens.spacing.sm))
                    .px(u(tokens.spacing.xl))
                    .py(u(tokens.spacing.sm))
                    .items_center()
                    .child(
                        div()
                            .debug_selector(|| "helm-release-manifest-note".into())
                            .text_size(u(tokens.font.small))
                            .text_color(tokens.colors.text_muted)
                            .child(if masked {
                                "Secret data is masked."
                            } else {
                                "Secret data is shown."
                            }),
                    )
                    .child(div().flex_1())
                    .child(self.reveal_button(!masked, cx)),
            )
            .child(
                div()
                    .debug_selector(|| "helm-release-manifest".into())
                    .flex_1()
                    .min_h_0()
                    .child(detail.manifest.code.clone()),
            )
            .into_any_element()
    }

    pub(super) fn notes_body(&self, detail: &ReleaseDetail, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        if detail.notes.empty {
            return self.note(
                "helm-release-notes-empty",
                "This chart has no notes.",
                false,
                cx,
            );
        }
        v_flex()
            .size_full()
            .child(
                div()
                    .debug_selector(|| "helm-release-notes".into())
                    .flex_1()
                    .min_h_0()
                    .child(detail.notes.code.clone()),
            )
            .when(!detail.links.is_empty(), |col| {
                col.child(
                    h_flex()
                        .debug_selector(|| "helm-release-links".into())
                        .flex_none()
                        .flex_wrap()
                        .gap(u(tokens.spacing.sm))
                        .px(u(tokens.spacing.xl))
                        .py(u(tokens.spacing.sm))
                        .border_t_1()
                        .border_color(tokens.colors.border_variant)
                        .children(detail.links.iter().enumerate().map(|(ix, url)| {
                            let url = url.clone();
                            Button::new(("helm-release-link", ix))
                                .ghost()
                                .xsmall()
                                .icon(Icon::new(IconName::ExternalLink).size(u(px(12.))))
                                .label(SharedString::from(url.clone()))
                                .on_click(move |_, _, cx| cx.open_url(&url))
                        })),
                )
            })
            .into_any_element()
    }
}
