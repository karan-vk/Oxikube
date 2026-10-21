//! The add-repository form.

use gpui::{
    AnyElement, Context, Hsla, InteractiveElement as _, IntoElement as _, ParentElement as _,
    Styled as _, div, px,
};
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::checkbox::Checkbox;
use oxikube_ui::input::Input;
use oxikube_ui::layout::{Disableable as _, h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Sizable as _, u};

use super::super::view::HelmReposView;

impl HelmReposView {
    pub(super) fn add_form(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let form = self.form(cx);
        let errors = form.errors(&self.snapshot);
        let warnings = form.warnings();
        let can_submit = form.can_submit(&self.snapshot) && !self.model.busy;
        let note = |id: &'static str, text: Option<String>, colour: Hsla| {
            text.map(|text| {
                div()
                    .debug_selector(move || id.into())
                    .text_size(u(tokens.font.small))
                    .text_color(colour)
                    .child(text)
            })
        };
        let field = |label: &'static str, id: &'static str, input: &gpui::Entity<_>| {
            v_flex()
                .gap(u(px(4.)))
                .flex_1()
                .child(
                    div()
                        .text_size(u(tokens.font.small))
                        .text_color(colors.text_muted)
                        .child(label),
                )
                .child(
                    div()
                        .debug_selector(move || id.into())
                        .child(Input::new(input)),
                )
        };
        v_flex()
            .debug_selector(|| "helm-repos-form".into())
            .gap(u(tokens.spacing.md))
            .p(u(tokens.spacing.lg))
            .rounded(u(tokens.radius.md))
            .border_1()
            .border_color(colors.border)
            .bg(colors.surface)
            .child(
                h_flex()
                    .gap(u(tokens.spacing.md))
                    .items_start()
                    .child(field("Name", "helm-repos-name", &self.name))
                    .child(field("URL", "helm-repos-url", &self.url)),
            )
            .children(note("helm-repos-name-problem", errors.name, colors.error))
            .children(note("helm-repos-url-problem", errors.url, colors.error))
            .child(
                h_flex()
                    .gap(u(tokens.spacing.md))
                    .items_start()
                    .child(field(
                        "User name (optional)",
                        "helm-repos-username",
                        &self.username,
                    ))
                    .child(field(
                        "Password or token (optional)",
                        "helm-repos-password",
                        &self.password,
                    )),
            )
            .children(note(
                "helm-repos-credentials-problem",
                errors.credentials,
                colors.error,
            ))
            .child(field("CA file (optional)", "helm-repos-ca", &self.ca_file))
            .child(
                div().debug_selector(|| "helm-repos-skip-tls".into()).child(
                    Checkbox::new("helm-repos-skip-tls-box")
                        .label("Skip TLS certificate verification")
                        .checked(self.skip_tls)
                        .on_click(cx.listener(|this, checked: &bool, _, cx| {
                            this.set_skip_tls(*checked, cx)
                        })),
                ),
            )
            .children(note(
                "helm-repos-tls-warning",
                warnings.tls.map(str::to_owned),
                colors.warning,
            ))
            .children(note(
                "helm-repos-http-warning",
                warnings.plain_http.map(str::to_owned),
                colors.warning,
            ))
            .child(
                div()
                    .debug_selector(|| "helm-repos-credentials-note".into())
                    .text_size(u(tokens.font.small))
                    .text_color(colors.text_muted)
                    .child(
                        "Helm saves the repository, including any password, in its own \
                         repositories.yaml. Oxikube keeps no copy of either.",
                    ),
            )
            .child(
                h_flex()
                    .gap(u(tokens.spacing.md))
                    .justify_end()
                    .child(
                        Button::new("helm-repos-cancel")
                            .small()
                            .label("Cancel")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.toggle_form(window, cx)),
                            ),
                    )
                    .child(
                        div().debug_selector(|| "helm-repos-submit".into()).child(
                            Button::new("helm-repos-submit-button")
                                .small()
                                .primary()
                                .label("Add")
                                .disabled(!can_submit)
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.submit(window, cx)),
                                ),
                        ),
                    ),
            )
            .into_any_element()
    }
}
