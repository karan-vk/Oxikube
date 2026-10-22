//! The frame of the install tab: header, guidance without `helm`, and the three columns (the
//! form, the values editor, the preview / confirmation / output).

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, Hsla, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, Styled as _, Window, div, px,
};
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::checkbox::Checkbox;
use oxikube_ui::input::Input;
use oxikube_ui::layout::{Disableable as _, h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Icon, IconName, Sizable as _, u};

use super::model::Stage;
use super::view::HelmInstallView;

/// How many versions the picker shows (newest first); the rest are a search away in the catalog.
const VERSIONS_SHOWN: usize = 12;
/// How many existing namespaces are offered under the field.
const NAMESPACES_SHOWN: usize = 8;

impl Render for HelmInstallView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let body = match &self.stage {
            Stage::Loading => div()
                .debug_selector(|| "helm-install-loading".into())
                .p(u(tokens.spacing.xl))
                .text_color(colors.text_muted)
                .child("Looking for helm and reading the chart's default values...")
                .into_any_element(),
            Stage::NoHelm => self.guidance(cx),
            _ => h_flex()
                .size_full()
                .items_start()
                .child(self.form_column(cx))
                .child(self.values_column(cx))
                .child(self.side_column(cx))
                .into_any_element(),
        };
        v_flex()
            .id("helm-install")
            .debug_selector(|| "helm-install".into())
            .track_focus(&self.focus)
            .size_full()
            .bg(colors.background)
            .text_color(colors.text)
            .text_size(u(tokens.font.body))
            .child(
                h_flex()
                    .flex_none()
                    .px(u(tokens.spacing.xl))
                    .py(u(tokens.spacing.lg))
                    .gap(u(tokens.spacing.md))
                    .items_center()
                    .border_b_1()
                    .border_color(colors.border_variant)
                    .child(Icon::new(IconName::Download).size(u(px(16.))))
                    .child(
                        div()
                            .text_size(u(tokens.font.heading))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(SharedString::from(format!(
                                "Install {}",
                                self.chart.reference()
                            ))),
                    )
                    .child(
                        div()
                            .debug_selector(|| "helm-install-cluster".into())
                            .text_color(colors.text_muted)
                            .child(SharedString::from(format!(
                                "into {}",
                                self.deps.cluster_label
                            ))),
                    ),
            )
            .child(div().flex_1().min_h_0().child(body))
    }
}

impl HelmInstallView {
    /// What to do without `helm`: installing needs the binary, so nothing else is offered.
    fn guidance(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        v_flex()
            .debug_selector(|| "helm-install-guidance".into())
            .m(u(tokens.spacing.xl))
            .gap(u(tokens.spacing.sm))
            .p(u(tokens.spacing.lg))
            .rounded(u(tokens.radius.md))
            .border_1()
            .border_color(colors.border_variant)
            .bg(colors.surface)
            .child(
                div()
                    .text_color(colors.warning)
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .child("The helm command was not found"),
            )
            .child(div().text_color(colors.text_muted).child(
                "Installing a chart runs your own helm (3.10 or newer). Install it from \
                 https://helm.sh/docs/intro/install/ or set its path in the helm.path setting. \
                 Releases already in the cluster can still be read without it.",
            ))
            .into_any_element()
    }

    fn form_column(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let form = self.form(cx);
        let errors = form.errors(self.namespaces.as_deref());
        let form_ok = errors.is_empty();
        let editable = self.stage.is_editable();
        let label = |text: &'static str| {
            div()
                .text_size(u(tokens.font.small))
                .text_color(colors.text_muted)
                .child(text)
        };
        let problem = |id: &'static str, text: Option<String>, colour: Hsla| {
            text.map(|text| {
                div()
                    .debug_selector(move || id.into())
                    .text_size(u(tokens.font.small))
                    .text_color(colour)
                    .child(text)
            })
        };
        let typed = form.namespace.trim().to_owned();
        let suggestions: Vec<String> = self
            .namespaces
            .iter()
            .flatten()
            .filter(|n| n.starts_with(&typed) && **n != typed)
            .take(NAMESPACES_SHOWN)
            .cloned()
            .collect();
        let new_namespace = form.namespace_is_new(self.namespaces.as_deref());
        v_flex()
            .debug_selector(|| "helm-install-form".into())
            .flex_none()
            .w(u(px(300.)))
            .h_full()
            .gap(u(tokens.spacing.sm))
            .p(u(tokens.spacing.lg))
            .border_r_1()
            .border_color(colors.border_variant)
            .child(label("Release name"))
            .child(
                div()
                    .debug_selector(|| "helm-install-release".into())
                    .child(Input::new(&self.release).disabled(!editable)),
            )
            .children(problem(
                "helm-install-release-problem",
                errors.release,
                colors.error,
            ))
            .child(label("Namespace"))
            .child(
                div()
                    .debug_selector(|| "helm-install-namespace".into())
                    .child(Input::new(&self.namespace).disabled(!editable)),
            )
            .child(h_flex().flex_wrap().gap(u(px(4.))).children(
                suggestions.into_iter().enumerate().map(|(i, name)| {
                    let pick = name.clone();
                    Button::new(("helm-install-ns", i))
                        .xsmall()
                        .ghost()
                        .label(name)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.pick_namespace(&pick, window, cx)
                        }))
                }),
            ))
            .child(
                div()
                    .debug_selector(|| "helm-install-create-ns".into())
                    .child(
                        Checkbox::new("helm-install-create-ns-box")
                            .label(if new_namespace {
                                "Create namespace (it does not exist)"
                            } else {
                                "Create namespace"
                            })
                            .checked(self.create_namespace)
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.set_create_namespace(*checked, cx)
                            })),
                    ),
            )
            .children(problem(
                "helm-install-namespace-problem",
                errors.namespace,
                colors.error,
            ))
            .child(label("Chart version"))
            .child(self.version_picker(cx))
            .child(label("Advanced"))
            .child(
                div().debug_selector(|| "helm-install-wait".into()).child(
                    Checkbox::new("helm-install-wait-box")
                        .label("Wait for resources to be ready")
                        .checked(self.wait)
                        .on_click(
                            cx.listener(|this, checked: &bool, _, cx| this.set_wait(*checked, cx)),
                        ),
                ),
            )
            .child(label("Timeout (seconds)"))
            .child(
                div()
                    .debug_selector(|| "helm-install-timeout".into())
                    .child(Input::new(&self.timeout).disabled(!editable)),
            )
            .children(problem(
                "helm-install-timeout-problem",
                errors.timeout,
                colors.error,
            ))
            .child(self.buttons(form_ok, cx))
            .into_any_element()
    }

    fn version_picker(&self, cx: &mut Context<Self>) -> AnyElement {
        let latest = std::iter::once(String::new());
        let versions = latest.chain(self.versions.iter().take(VERSIONS_SHOWN).cloned());
        let mut known: Vec<String> = versions.collect();
        if !self.version.is_empty() && !known.contains(&self.version) {
            known.push(self.version.clone());
        }
        h_flex()
            .debug_selector(|| "helm-install-versions".into())
            .flex_wrap()
            .gap(u(px(4.)))
            .children(known.into_iter().enumerate().map(|(i, version)| {
                let label = if version.is_empty() {
                    "latest".to_owned()
                } else {
                    version.clone()
                };
                let selected = version == self.version;
                let button = Button::new(("helm-install-version", i))
                    .xsmall()
                    .label(label)
                    .on_click(cx.listener(move |this, _, _, cx| this.set_version(&version, cx)));
                div()
                    .debug_selector(move || format!("helm-install-version:{i}"))
                    .child(if selected {
                        button.primary()
                    } else {
                        button.ghost()
                    })
            }))
            .into_any_element()
    }

    fn buttons(&self, form_ok: bool, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let editable = self.stage.is_editable();
        let can_install = editable
            && self
                .fresh_preview(cx)
                .is_some_and(|shown| shown.preview.can_install());
        let previewing = self.stage == Stage::Previewing;
        h_flex()
            .mt(u(tokens.spacing.md))
            .gap(u(tokens.spacing.sm))
            .child(
                div()
                    .debug_selector(|| "helm-install-preview".into())
                    .child(
                        Button::new("helm-install-preview-button")
                            .label(if previewing {
                                "Previewing..."
                            } else {
                                "Preview"
                            })
                            .disabled(!editable || !form_ok)
                            .on_click(cx.listener(|this, _, window, cx| this.preview(window, cx))),
                    ),
            )
            .child(
                div()
                    .debug_selector(|| "helm-install-install".into())
                    .child(
                        Button::new("helm-install-install-button")
                            .primary()
                            .label("Install")
                            .disabled(!can_install)
                            .on_click(cx.listener(|this, _, _, cx| this.install(cx))),
                    ),
            )
            .when(
                matches!(self.stage, Stage::Installed(_) | Stage::Failed(_)),
                |row| {
                    row.child(
                        Button::new("helm-install-edit-again")
                            .ghost()
                            .label("Edit")
                            .on_click(cx.listener(|this, _, _, cx| this.edit_again(cx))),
                    )
                },
            )
            .into_any_element()
    }

    fn values_column(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .border_r_1()
            .border_color(colors.border_variant)
            .child(
                h_flex()
                    .flex_none()
                    .px(u(tokens.spacing.lg))
                    .py(u(tokens.spacing.sm))
                    .gap(u(tokens.spacing.sm))
                    .child(div().font_weight(gpui::FontWeight::MEDIUM).child("Values"))
                    .child(
                        div()
                            .text_size(u(tokens.font.small))
                            .text_color(colors.text_muted)
                            .child(
                                "The chart's defaults; hover a key for its docs. Kept in memory \
                                 only.",
                            ),
                    ),
            )
            .when_some(self.defaults_problem.clone(), |column, why| {
                column.child(
                    div()
                        .debug_selector(|| "helm-install-defaults-problem".into())
                        .px(u(tokens.spacing.lg))
                        .text_size(u(tokens.font.small))
                        .text_color(colors.error)
                        .child(SharedString::from(format!(
                            "The default values could not be read: {why}"
                        ))),
                )
            })
            .child(
                div()
                    .debug_selector(|| "helm-install-values".into())
                    .flex_1()
                    .min_h_0()
                    .child(self.editor.clone()),
            )
            .into_any_element()
    }
}
