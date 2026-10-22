//! The review column while editing: the tabs (values diff, chart defaults, preview), the diffs as
//! virtualised rows of the E10 editor's diff, and the preview (objects that change, the manifest).

use std::ops::Range;
use std::sync::Arc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    Styled as _, div, px, uniform_list,
};
use oxikube_domain::helm::ObjectKey;
use oxikube_editor::diff::ManifestDiff;
use oxikube_editor::view::{diff_inline_rows, diff_summary};
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::layout::{Scrollbar, h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Sizable as _, u};

use super::model::{Pane, Stage};
use super::view::HelmUpgradeView;

/// How many object names a changes group lists before "and N more".
const NAMES_SHOWN: usize = 40;

impl HelmUpgradeView {
    /// The tabs (values diff, chart defaults, preview) and the one shown.
    pub(super) fn review_pane(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let tab = |pane: Pane, id: &'static str, text: &'static str, cx: &mut Context<Self>| {
            let button = Button::new(id)
                .xsmall()
                .label(text)
                .on_click(cx.listener(move |this, _, _, cx| this.set_pane(pane, cx)));
            div()
                .debug_selector(move || id.into())
                .child(if self.pane == pane {
                    button.primary()
                } else {
                    button.ghost()
                })
        };
        let body = match self.pane {
            Pane::ValuesDiff => self.diff_pane(
                "values",
                self.values_diff.clone(),
                "The values on the left against the release's current values. Secret-looking \
                 values show as fingerprints.",
                cx,
            ),
            Pane::ChartDefaults => self.diff_pane(
                "defaults",
                self.defaults_diff.clone(),
                "The chart's default values at the version the release runs against the target \
                 version: keys the upgrade adds, renames or drops.",
                cx,
            ),
            Pane::Preview => self.preview_pane(cx),
        };
        v_flex()
            .debug_selector(|| "helm-upgrade-review".into())
            .size_full()
            .child(
                h_flex()
                    .flex_none()
                    .px(u(tokens.spacing.lg))
                    .py(u(tokens.spacing.sm))
                    .gap(u(tokens.spacing.sm))
                    .border_b_1()
                    .border_color(colors.border_variant)
                    .child(tab(
                        Pane::ValuesDiff,
                        "helm-upgrade-tab-values",
                        "Values diff",
                        cx,
                    ))
                    .child(tab(
                        Pane::ChartDefaults,
                        "helm-upgrade-tab-defaults",
                        "Chart defaults",
                        cx,
                    ))
                    .child(tab(
                        Pane::Preview,
                        "helm-upgrade-tab-preview",
                        "Preview",
                        cx,
                    )),
            )
            .child(div().flex_1().min_h_0().child(body))
            .into_any_element()
    }

    fn diff_pane(
        &self,
        which: &'static str,
        diff: Option<Arc<ManifestDiff>>,
        help: &'static str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let (summary, rows) = match &diff {
            None => ("Reading...".to_owned(), 0),
            Some(diff) => (diff_summary(diff), diff.inline_rows().len()),
        };
        let pane = which == "values";
        v_flex()
            .debug_selector(move || format!("helm-upgrade-diff-{which}"))
            .size_full()
            .child(
                v_flex()
                    .flex_none()
                    .px(u(tokens.spacing.lg))
                    .py(u(tokens.spacing.sm))
                    .gap(u(px(2.)))
                    .child(
                        div()
                            .debug_selector(move || format!("helm-upgrade-diff-{which}-summary"))
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .child(SharedString::from(summary)),
                    )
                    .child(
                        div()
                            .text_size(u(tokens.font.small))
                            .text_color(colors.text_muted)
                            .child(help),
                    ),
            )
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .border_t_1()
                    .border_color(colors.border_variant)
                    .font_family(cx.mono_font_family())
                    .text_size(u(tokens.font.small))
                    .child(
                        uniform_list(
                            if pane {
                                "helm-upgrade-values-diff"
                            } else {
                                "helm-upgrade-defaults-diff"
                            },
                            rows,
                            cx.processor(move |this, range, _, cx| this.diff_rows(pane, range, cx)),
                        )
                        .track_scroll(&self.diff_scroll)
                        .size_full(),
                    )
                    .child(Scrollbar::vertical(&self.diff_scroll)),
            )
            .into_any_element()
    }

    fn diff_rows(
        &mut self,
        values: bool,
        range: Range<usize>,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let diff = if values {
            &self.values_diff
        } else {
            &self.defaults_diff
        };
        let Some(diff) = diff.as_ref() else {
            return Vec::new();
        };
        let end = range.end.min(diff.inline_rows().len());
        diff_inline_rows(diff, range.start.min(end)..end, cx)
    }

    fn preview_pane(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let fresh = self.fresh_preview(cx).is_some();
        let headline = match (&self.shown, &self.preview_error, &self.stage) {
            (_, _, Stage::Previewing) => (
                "Rendering the chart and asking the server...".to_owned(),
                colors.info,
            ),
            (_, Some(_), _) => ("The preview failed".to_owned(), colors.error),
            (Some(shown), _, _) if !fresh => (
                format!(
                    "{} (out of date: preview again)",
                    shown.preview.changes.describe()
                ),
                colors.warning,
            ),
            (Some(shown), _, _) => (shown.preview.changes.describe(), colors.text),
            (None, None, _) => (
                "Preview renders the new chart with your values, has the server check it and \
                 lists the objects it adds, removes and changes; nothing is upgraded."
                    .to_owned(),
                colors.text_muted,
            ),
        };
        let server = self
            .shown
            .as_ref()
            .and_then(|s| s.preview.server_error.clone());
        let changes = self.shown.as_ref().map(|s| s.preview.changes.clone());
        v_flex()
            .debug_selector(|| "helm-upgrade-preview-pane".into())
            .size_full()
            .child(
                v_flex()
                    .flex_none()
                    .px(u(tokens.spacing.lg))
                    .py(u(tokens.spacing.sm))
                    .gap(u(px(2.)))
                    .child(
                        div()
                            .debug_selector(|| "helm-upgrade-summary".into())
                            .text_color(headline.1)
                            .child(SharedString::from(headline.0)),
                    )
                    .when_some(self.preview_error.clone(), |pane, why| {
                        pane.child(
                            div()
                                .debug_selector(|| "helm-upgrade-preview-error".into())
                                .text_size(u(tokens.font.small))
                                .text_color(colors.error)
                                .child(SharedString::from(why)),
                        )
                    })
                    .when_some(server, |pane, why| {
                        pane.child(
                            div()
                                .debug_selector(|| "helm-upgrade-server-error".into())
                                .text_size(u(tokens.font.small))
                                .text_color(colors.error)
                                .child(SharedString::from(format!("The server refused it: {why}"))),
                        )
                    })
                    .when_some(changes, |pane, changes| {
                        pane.child(
                            v_flex()
                                .debug_selector(|| "helm-upgrade-changes".into())
                                .text_size(u(tokens.font.small))
                                .child(names("Added", &changes.added, colors.success))
                                .child(names("Changed", &changes.changed, colors.warning))
                                .child(names("Removed", &changes.removed, colors.error)),
                        )
                    }),
            )
            .when(self.shown.is_some(), |pane| {
                pane.child(
                    div()
                        .debug_selector(|| "helm-upgrade-manifest".into())
                        .flex_1()
                        .min_h_0()
                        .border_t_1()
                        .border_color(colors.border_variant)
                        .child(oxikube_ui::editor::code_view(&self.manifest).size_full()),
                )
            })
            .into_any_element()
    }
}

/// One group of the changes: `Added (2): Ingress web, Secret web-extra`.
fn names(title: &'static str, keys: &[ObjectKey], colour: gpui::Hsla) -> impl IntoElement {
    let mut text = keys
        .iter()
        .take(NAMES_SHOWN)
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    if keys.len() > NAMES_SHOWN {
        text.push_str(&format!(" and {} more", keys.len() - NAMES_SHOWN));
    }
    div()
        .debug_selector(move || format!("helm-upgrade-changes-{}", title.to_lowercase()))
        .when(!keys.is_empty(), |d| {
            d.text_color(colour).child(SharedString::from(format!(
                "{title} ({}): {text}",
                keys.len()
            )))
        })
}
