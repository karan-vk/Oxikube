//! The left columns of the upgrade tab: the form (chart, version, `--reuse-values`, advanced,
//! buttons) and the values editor.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, Hsla, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, Styled as _, div, px,
};
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::checkbox::Checkbox;
use oxikube_ui::input::Input;
use oxikube_ui::layout::{Disableable as _, h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Sizable as _, u};

use super::model::Stage;
use super::view::HelmUpgradeView;

/// How many versions the picker shows (newest first); the rest are a search away in the catalog.
const VERSIONS_SHOWN: usize = 12;

impl HelmUpgradeView {
    pub(super) fn form_column(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let form = self.form(cx);
        let errors = form.errors();
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
        let sources = self.sources.clone();
        v_flex()
            .debug_selector(|| "helm-upgrade-form".into())
            .flex_none()
            .w(u(px(300.)))
            .h_full()
            .gap(u(tokens.spacing.sm))
            .p(u(tokens.spacing.lg))
            .border_r_1()
            .border_color(colors.border_variant)
            .child(label("Chart"))
            .child(
                div()
                    .debug_selector(|| "helm-upgrade-chart".into())
                    .child(Input::new(&self.chart).disabled(!editable)),
            )
            .child(
                h_flex()
                    .debug_selector(|| "helm-upgrade-sources".into())
                    .flex_wrap()
                    .gap(u(px(4.)))
                    .children(sources.into_iter().enumerate().map(|(i, source)| {
                        let selected = source.chart.reference() == form.chart.trim();
                        let button = Button::new(("helm-upgrade-source", i))
                            .xsmall()
                            .label(source.chart.reference())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.pick_source(&source, window, cx);
                            }));
                        if selected {
                            button.primary()
                        } else {
                            button.ghost()
                        }
                    })),
            )
            .children(problem(
                "helm-upgrade-chart-problem",
                errors.chart,
                colors.error,
            ))
            .child(label("Chart version"))
            .child(self.version_picker(cx))
            .child(label("Values"))
            .child(
                div().debug_selector(|| "helm-upgrade-reuse".into()).child(
                    Checkbox::new("helm-upgrade-reuse-box")
                        .label("Reuse the previous values (--reuse-values)")
                        .checked(self.reuse_values)
                        .on_click(cx.listener(|this, checked: &bool, _, cx| {
                            this.set_reuse_values(*checked, cx)
                        })),
                ),
            )
            .child(
                div()
                    .debug_selector(|| "helm-upgrade-reuse-help".into())
                    .text_size(u(tokens.font.small))
                    .text_color(colors.text_muted)
                    .child(if self.reuse_values {
                        "Keeps the previous revision's values and ignores new defaults of the \
                         chart, which can break an upgrade when the chart renamed or added keys."
                    } else {
                        "Off: the values on the right are sent in full, over the new chart's \
                         defaults."
                    }),
            )
            .child(label("Advanced"))
            .child(
                div().debug_selector(|| "helm-upgrade-wait".into()).child(
                    Checkbox::new("helm-upgrade-wait-box")
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
                    .debug_selector(|| "helm-upgrade-timeout".into())
                    .child(Input::new(&self.timeout).disabled(!editable)),
            )
            .children(problem(
                "helm-upgrade-timeout-problem",
                errors.timeout,
                colors.error,
            ))
            .child(self.buttons(form_ok, cx))
            .into_any_element()
    }

    /// "latest", the newest versions of the chosen chart and the version the release runs.
    fn version_picker(&self, cx: &mut Context<Self>) -> AnyElement {
        let form = self.form(cx);
        let chosen = self
            .sources
            .iter()
            .find(|s| s.chart.reference() == form.chart.trim());
        let mut known: Vec<String> = std::iter::once(String::new())
            .chain(
                chosen
                    .into_iter()
                    .flat_map(|s| s.versions.iter().take(VERSIONS_SHOWN).cloned()),
            )
            .collect();
        if !self.version.is_empty() && !known.contains(&self.version) {
            known.push(self.version.clone());
        }
        let running = self.current.as_ref().map(|c| c.chart.version.clone());
        h_flex()
            .debug_selector(|| "helm-upgrade-versions".into())
            .flex_wrap()
            .gap(u(px(4.)))
            .children(known.into_iter().enumerate().map(|(i, version)| {
                let mut label = if version.is_empty() {
                    "latest".to_owned()
                } else {
                    version.clone()
                };
                if running.as_deref() == Some(version.as_str()) {
                    label.push_str(" (current)");
                }
                let selected = version == self.version;
                let button = Button::new(("helm-upgrade-version", i))
                    .xsmall()
                    .label(label)
                    .on_click(cx.listener(move |this, _, _, cx| this.set_version(&version, cx)));
                div()
                    .debug_selector(move || format!("helm-upgrade-version:{i}"))
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
        let can_upgrade = editable
            && self
                .fresh_preview(cx)
                .is_some_and(|shown| shown.preview.can_upgrade());
        let previewing = self.stage == Stage::Previewing;
        h_flex()
            .mt(u(tokens.spacing.md))
            .gap(u(tokens.spacing.sm))
            .child(
                div()
                    .debug_selector(|| "helm-upgrade-preview".into())
                    .child(
                        Button::new("helm-upgrade-preview-button")
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
                    .debug_selector(|| "helm-upgrade-upgrade".into())
                    .child(
                        Button::new("helm-upgrade-upgrade-button")
                            .primary()
                            .label("Upgrade")
                            .disabled(!can_upgrade)
                            .on_click(cx.listener(|this, _, _, cx| this.upgrade(cx))),
                    ),
            )
            .when(
                matches!(self.stage, Stage::Upgraded(_) | Stage::Failed(_)),
                |row| {
                    row.child(
                        Button::new("helm-upgrade-edit-again")
                            .ghost()
                            .label("Edit")
                            .on_click(cx.listener(|this, _, _, cx| this.edit_again(cx))),
                    )
                },
            )
            .into_any_element()
    }

    pub(super) fn values_column(&self, cx: &mut Context<Self>) -> AnyElement {
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
                                "The release's current values; hover a key for its docs. Kept \
                                 in memory only.",
                            ),
                    ),
            )
            .when_some(self.defaults_problem.clone(), |column, why| {
                column.child(
                    div()
                        .debug_selector(|| "helm-upgrade-defaults-problem".into())
                        .px(u(tokens.spacing.lg))
                        .text_size(u(tokens.font.small))
                        .text_color(colors.warning)
                        .child(SharedString::from(format!(
                            "The chart's default values could not be read: {why}"
                        ))),
                )
            })
            .child(
                div()
                    .debug_selector(|| "helm-upgrade-values".into())
                    .flex_1()
                    .min_h_0()
                    .child(self.editor.clone()),
            )
            .into_any_element()
    }
}
