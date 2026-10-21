//! The frame of the repositories page.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, Hsla, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::checkbox::Checkbox;
use oxikube_ui::input::Input;
use oxikube_ui::layout::{Disableable as _, h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Colors, Icon, IconName, Sizable as _, Tokens, u};

use super::model::{RowModel, Tone};
use super::view::HelmReposView;

fn tone_colour(tone: Tone, colors: &Colors) -> Hsla {
    match tone {
        Tone::Neutral => colors.text,
        Tone::Good => colors.success,
        Tone::Pending => colors.info,
        Tone::Error => colors.error,
        Tone::Muted => colors.text_muted,
    }
}

impl Render for HelmReposView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let header = self.header(cx);
        let body = if !self.model.checked {
            div()
                .debug_selector(|| "helm-repos-checking".into())
                .text_color(colors.text_muted)
                .child("Looking for helm...")
                .into_any_element()
        } else if self.model.cli_available {
            v_flex()
                .gap(u(tokens.spacing.lg))
                .children(self.banner(&tokens))
                .when(self.form_open, |body| body.child(self.add_form(cx)))
                .children(self.suggestion_chips(cx))
                .child(self.list(cx))
                .into_any_element()
        } else {
            guidance(&tokens)
        };
        v_flex()
            .id("helm-repos")
            .debug_selector(|| "helm-repos".into())
            .track_focus(&self.focus)
            .size_full()
            .bg(colors.background)
            .text_color(colors.text)
            .text_size(u(tokens.font.body))
            .child(header)
            .child(
                v_flex()
                    .id("helm-repos-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .px(u(tokens.spacing.xl))
                    .py(u(tokens.spacing.lg))
                    .child(body),
            )
    }
}

/// What to do when there is no `helm`.
fn guidance(tokens: &Tokens) -> AnyElement {
    let colors = tokens.colors;
    v_flex()
        .debug_selector(|| "helm-repos-guidance".into())
        .gap(u(tokens.spacing.sm))
        .p(u(tokens.spacing.lg))
        .rounded(u(tokens.radius.md))
        .border_1()
        .border_color(colors.border_variant)
        .bg(colors.surface)
        .child(
            h_flex()
                .gap(u(tokens.spacing.md))
                .items_center()
                .text_color(colors.warning)
                .child(
                    Icon::new(IconName::CircleAlert)
                        .size(u(px(16.)))
                        .color(colors.warning),
                )
                .child(
                    div()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .child("The helm command was not found"),
                ),
        )
        .child(div().text_color(colors.text_muted).child(
            "Chart repositories are managed with the helm command. Install Helm 3.10 or newer \
             (for example `brew install helm`), or set its location with the helm.path setting, \
             then restart Oxikube.",
        ))
        .child(
            div()
                .text_color(colors.text_muted)
                .child("Releases, their history and values can still be read without it."),
        )
        .into_any_element()
}

impl HelmReposView {
    fn header(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let enabled = self.model.cli_available;
        let busy = self.model.busy;
        let can_update = enabled && !busy && !self.model.rows.is_empty();
        h_flex()
            .flex_none()
            .gap(u(tokens.spacing.lg))
            .px(u(tokens.spacing.xl))
            .py(u(tokens.spacing.lg))
            .items_center()
            .border_b_1()
            .border_color(colors.border_variant)
            .child(
                div()
                    .text_size(u(tokens.font.heading))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child("Helm Repositories"),
            )
            .child(
                div()
                    .text_color(colors.text_muted)
                    .child("Stored in Helm's own configuration"),
            )
            .child(div().flex_1())
            .when(enabled, |header| {
                header
                    .child(
                        div()
                            .debug_selector(|| "helm-repos-update-all".into())
                            .child(
                                Button::new("helm-repos-update-all")
                                    .small()
                                    .icon(Icon::new(IconName::RefreshCw).size(u(px(14.))))
                                    .label("Update all")
                                    .tooltip("Refresh the index of every repository")
                                    .disabled(!can_update)
                                    .on_click(cx.listener(|this, _, _, cx| this.update_all(cx))),
                            ),
                    )
                    .child(
                        div()
                            .debug_selector(|| "helm-repos-add-toggle".into())
                            .child(
                                Button::new("helm-repos-add-toggle")
                                    .small()
                                    .primary()
                                    .icon(Icon::new(IconName::Plus).size(u(px(14.))))
                                    .label("Add repository")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.toggle_form(window, cx)
                                    })),
                            ),
                    )
            })
            .into_any_element()
    }

    /// Progress, the last failure and the last success.
    fn banner(&self, tokens: &Tokens) -> Vec<AnyElement> {
        let colors = tokens.colors;
        let line = |id: &'static str, text: &str, colour: Hsla| {
            div()
                .debug_selector(move || id.into())
                .text_color(colour)
                .child(SharedString::from(text.to_owned()))
                .into_any_element()
        };
        let mut lines = Vec::new();
        if let Some(text) = &self.model.progress {
            lines.push(line("helm-repos-progress", text, colors.info));
        }
        if let Some(text) = &self.model.problem {
            lines.push(line("helm-repos-problem", text, colors.error));
        }
        if let Some(text) = &self.model.notice {
            lines.push(line("helm-repos-notice", text, colors.success));
        }
        lines
    }

    fn suggestion_chips(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.model.suggestions.is_empty() || !self.model.loaded {
            return None;
        }
        let tokens = cx.tokens();
        let mut chips = h_flex()
            .gap(u(tokens.spacing.sm))
            .items_center()
            .flex_wrap()
            .child(
                div()
                    .text_color(tokens.colors.text_muted)
                    .child("Quick add:"),
            );
        for (index, suggestion) in self.model.suggestions.iter().enumerate() {
            chips = chips.child(
                div()
                    .debug_selector(move || format!("helm-repos-suggestion:{index}"))
                    .child(
                        Button::new(("helm-repos-suggestion", index))
                            .xsmall()
                            .outline()
                            .label(suggestion.name)
                            .tooltip(suggestion.url)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.use_suggestion(index, window, cx)
                            })),
                    ),
            );
        }
        Some(chips.into_any_element())
    }

    fn add_form(&self, cx: &mut Context<Self>) -> AnyElement {
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

    fn list(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        if self.model.rows.is_empty() {
            let text = if self.model.loaded {
                "No chart repositories yet. Add one above, or pick a suggestion."
            } else {
                "Reading Helm's repositories..."
            };
            return div()
                .debug_selector(|| "helm-repos-empty".into())
                .text_color(colors.text_muted)
                .child(text)
                .into_any_element();
        }
        // A machine has a handful of repositories, so the rows are plain children.
        let mut list = v_flex().debug_selector(|| "helm-repos-list".into());
        for (index, row) in self.model.rows.iter().enumerate() {
            list = list.child(self.row(index, row, cx));
        }
        list.into_any_element()
    }

    fn row(&self, index: usize, row: &RowModel, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let busy = self.model.busy;
        let confirming = self.confirm_remove.as_deref() == Some(row.name.as_str());
        let (update_name, remove_name) = (row.name.clone(), row.name.clone());
        let (row_sel, status_sel) = (
            format!("helm-repo-row:{}", row.name),
            format!("helm-repo-status:{}:{}", row.name, tone_name(row.tone)),
        );
        h_flex()
            .id(("helm-repo", index))
            .debug_selector(move || row_sel)
            .gap(u(tokens.spacing.lg))
            .py(u(tokens.spacing.sm))
            .items_start()
            .border_b_1()
            .border_color(colors.border_variant)
            .child(
                div()
                    .w(u(px(180.)))
                    .flex_none()
                    .truncate()
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .child(SharedString::from(row.name.clone())),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(u(px(2.)))
                    .child(
                        div()
                            .truncate()
                            .text_color(colors.text_muted)
                            .child(SharedString::from(row.url.clone())),
                    )
                    // The reason a refresh failed can be a sentence: it wraps instead of cutting.
                    .child(
                        div()
                            .debug_selector(move || status_sel)
                            .text_size(u(tokens.font.small))
                            .text_color(tone_colour(row.tone, &colors))
                            .child(SharedString::from(row.status.clone())),
                    ),
            )
            .child(
                div()
                    .debug_selector({
                        let name = row.name.clone();
                        move || format!("helm-repo-update:{name}")
                    })
                    .child(
                        Button::new(("helm-repo-update", index))
                            .xsmall()
                            .outline()
                            .label("Update")
                            .disabled(busy)
                            .on_click(
                                cx.listener(move |this, _, _, cx| {
                                    this.update_one(&update_name, cx)
                                }),
                            ),
                    ),
            )
            .child(
                div()
                    .debug_selector({
                        let name = row.name.clone();
                        move || format!("helm-repo-remove:{name}")
                    })
                    .child(
                        Button::new(("helm-repo-remove", index))
                            .xsmall()
                            .ghost()
                            .label(if confirming {
                                "Click again to remove"
                            } else {
                                "Remove"
                            })
                            .disabled(busy)
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.remove(&remove_name, cx)),
                            ),
                    ),
            )
            .into_any_element()
    }
}

fn tone_name(tone: Tone) -> &'static str {
    match tone {
        Tone::Neutral => "neutral",
        Tone::Good => "good",
        Tone::Pending => "pending",
        Tone::Error => "error",
        Tone::Muted => "muted",
    }
}
