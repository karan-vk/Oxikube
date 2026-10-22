//! The frame of the upgrade tab: header, guidance without `helm`, the release picker and the three
//! columns (the form, the values editor, the diffs / preview / confirmation / output).

use std::ops::Range;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div, px, uniform_list,
};
use oxikube_ui::layout::{Scrollbar, h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Icon, IconName, u};

use super::model::Stage;
use super::view::HelmUpgradeView;

/// A release picker row's height at 100 % zoom (uniform, so the list is virtual).
const PICKER_ROW_HEIGHT: f32 = 26.0;

impl Render for HelmUpgradeView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let body = match &self.stage {
            Stage::Loading => note(cx, "helm-upgrade-loading", "Reading the release..."),
            Stage::NoHelm => self.guidance(cx),
            Stage::Choosing => self.picker(cx),
            Stage::Failed(why) if self.current.is_none() => {
                let why = why.clone();
                v_flex()
                    .debug_selector(|| "helm-upgrade-load-error".into())
                    .m(u(tokens.spacing.xl))
                    .text_color(colors.error)
                    .child(SharedString::from(why))
                    .into_any_element()
            }
            _ => h_flex()
                .size_full()
                .items_start()
                .child(self.form_column(cx))
                .child(self.values_column(cx))
                .child(self.side_column(cx))
                .into_any_element(),
        };
        let title = match &self.target {
            Some(release) => format!("Upgrade {release}"),
            None => "Upgrade a release".to_owned(),
        };
        v_flex()
            .id("helm-upgrade")
            .debug_selector(|| "helm-upgrade".into())
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
                            .child(SharedString::from(title)),
                    )
                    .child(
                        div()
                            .debug_selector(|| "helm-upgrade-cluster".into())
                            .text_color(colors.text_muted)
                            .child(SharedString::from(format!(
                                "on {}",
                                self.deps.cluster_label
                            ))),
                    )
                    .when_some(self.current_line(), |row, line| {
                        row.child(
                            div()
                                .debug_selector(|| "helm-upgrade-current".into())
                                .text_color(colors.text_muted)
                                .child(SharedString::from(line)),
                        )
                    }),
            )
            .child(div().flex_1().min_h_0().child(body))
    }
}

fn note(cx: &Context<HelmUpgradeView>, selector: &'static str, text: &'static str) -> AnyElement {
    let tokens = cx.tokens();
    div()
        .debug_selector(move || selector.into())
        .p(u(tokens.spacing.xl))
        .text_color(tokens.colors.text_muted)
        .child(text)
        .into_any_element()
}

impl HelmUpgradeView {
    /// `nginx-15.0.0, app 1.25.3, revision 3 (deployed)`: what the release runs now.
    fn current_line(&self) -> Option<String> {
        let current = self.current.as_ref()?;
        let app = current
            .app_version
            .as_ref()
            .map_or_else(String::new, |v| format!(", app {v}"));
        Some(format!(
            "runs {}{app}, revision {} ({})",
            current.chart, current.revision, current.status
        ))
    }

    /// What to do without `helm`: upgrading needs the binary, so nothing else is offered.
    fn guidance(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        v_flex()
            .debug_selector(|| "helm-upgrade-guidance".into())
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
                "Upgrading a release runs your own helm (3.10 or newer). Install it from \
                 https://helm.sh/docs/intro/install/ or set its path in the helm.path setting. \
                 The release's values, history and manifest can still be read without it.",
            ))
            .into_any_element()
    }

    /// The cluster's releases to pick from, virtualised.
    fn picker(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let Some(count) = self.releases.as_ref().map(Vec::len) else {
            return note(
                cx,
                "helm-upgrade-releases-loading",
                "Reading the releases...",
            );
        };
        if count == 0 {
            return note(
                cx,
                "helm-upgrade-no-releases",
                "This cluster has no Helm releases.",
            );
        }
        v_flex()
            .debug_selector(|| "helm-upgrade-picker".into())
            .size_full()
            .child(
                div()
                    .flex_none()
                    .px(u(tokens.spacing.xl))
                    .py(u(tokens.spacing.sm))
                    .text_color(colors.text_muted)
                    .child("Pick the release to upgrade."),
            )
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(
                        uniform_list(
                            "helm-upgrade-releases",
                            count,
                            cx.processor(|this, range, _, cx| this.release_rows(range, cx)),
                        )
                        .track_scroll(&self.releases_scroll)
                        .size_full(),
                    )
                    .child(Scrollbar::vertical(&self.releases_scroll)),
            )
            .into_any_element()
    }

    fn release_rows(&mut self, range: Range<usize>, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let tokens = cx.tokens();
        let Some(releases) = self.releases.as_ref() else {
            return Vec::new();
        };
        range
            .filter_map(|i| releases.get(i).map(|r| (i, r.clone())))
            .map(|(i, release)| {
                let reference = release.release.clone();
                let label = format!(
                    "{}  {}  {}  rev {}  {}",
                    release.release, release.chart, release.status, release.revision, ""
                );
                h_flex()
                    .id(("helm-upgrade-release", i))
                    .debug_selector(move || format!("helm-upgrade-release:{i}"))
                    .h(u(px(PICKER_ROW_HEIGHT)))
                    .px(u(tokens.spacing.xl))
                    .items_center()
                    .hover(|row| row.bg(tokens.colors.surface))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.pick_release(reference.clone(), window, cx);
                    }))
                    .child(SharedString::from(label.trim_end().to_owned()))
                    .into_any_element()
            })
            .collect()
    }
}
