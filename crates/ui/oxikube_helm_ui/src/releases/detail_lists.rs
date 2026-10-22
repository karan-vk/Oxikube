//! The detail pane's Resources and History tabs: virtualised lists.

use std::ops::Range;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, div, px, uniform_list,
};
use jiff::Timestamp;
use oxikube_domain::command::Command;
use oxikube_ui::layout::{Scrollbar, h_flex};
use oxikube_ui::{ActiveTokens as _, Icon, IconName, u};

use super::backend::{ObjectRow, ObjectState};
use super::delegate::{tone_colour, tone_icon};
use super::detail::ReleaseDetail;
use super::model::{age, tone};
use super::view::HelmReleasesView;

const ROW_HEIGHT: f32 = 28.0;

impl HelmReleasesView {
    pub(super) fn resources_body(
        &self,
        detail: &ReleaseDetail,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if detail.resources.objects.is_empty() {
            return self.note(
                "helm-release-resources-empty",
                "This revision's manifest has no objects.",
                false,
                cx,
            );
        }
        div()
            .relative()
            .size_full()
            .debug_selector(|| "helm-release-resources".into())
            .child(
                uniform_list(
                    "helm-release-resources-list",
                    detail.resources.objects.len(),
                    cx.processor(|this, range, _, cx| this.object_rows(range, cx)),
                )
                .track_scroll(&detail.resources.scroll)
                .size_full(),
            )
            .child(Scrollbar::vertical(&detail.resources.scroll))
            .into_any_element()
    }

    pub(super) fn object_rows(
        &mut self,
        range: Range<usize>,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let Some(detail) = &self.detail else {
            return Vec::new();
        };
        range
            .filter_map(|ix| {
                let row: &ObjectRow = detail.resources.objects.get(ix)?;
                let (icon, colour, label) = match row.state {
                    ObjectState::Healthy => (IconName::CircleCheck, colors.success, "healthy"),
                    ObjectState::Unhealthy => (IconName::CircleAlert, colors.error, "unhealthy"),
                    ObjectState::Present => (IconName::Circle, colors.text_muted, "present"),
                    ObjectState::Missing => (IconName::CircleX, colors.error, "missing"),
                    ObjectState::Loading => (IconName::LoaderCircle, colors.text_muted, "..."),
                    ObjectState::NotServed => {
                        (IconName::CircleAlert, colors.warning, "kind not served")
                    }
                };
                let target = row.reference.clone();
                let openable = matches!(
                    row.state,
                    ObjectState::Healthy | ObjectState::Unhealthy | ObjectState::Present
                );
                let selector = format!(
                    "helm-release-object:{}/{}",
                    row.reference.gvk.kind, row.reference.name
                );
                let place = row
                    .reference
                    .namespace()
                    .map(|ns| format!("{ns}  "))
                    .unwrap_or_default();
                Some(
                    h_flex()
                        .id(("helm-release-object", ix))
                        .debug_selector(move || selector)
                        .h(u(px(ROW_HEIGHT)))
                        .px(u(tokens.spacing.xl))
                        .gap(u(tokens.spacing.lg))
                        .items_center()
                        .when(openable, |row| {
                            row.cursor_pointer()
                                .hover(|row| row.bg(colors.element_hover))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.deps.dispatcher.dispatch(
                                        Command::ResourceOpen {
                                            target: target.clone(),
                                        },
                                        cx,
                                    );
                                }))
                        })
                        .child(Icon::new(icon).size(u(px(13.))).color(colour))
                        .child(
                            div()
                                .w(u(px(150.)))
                                .flex_none()
                                .text_color(colors.text_muted)
                                .truncate()
                                .child(SharedString::from(row.reference.gvk.kind.to_string())),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .child(SharedString::from(format!(
                                    "{place}{}",
                                    row.reference.name
                                ))),
                        )
                        .child(div().flex_none().text_color(colour).child(label))
                        .into_any_element(),
                )
            })
            .collect()
    }

    pub(super) fn history_body(
        &self,
        detail: &ReleaseDetail,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if detail.history.revisions.is_empty() {
            return self.note(
                "helm-release-history-empty",
                "No revisions found.",
                false,
                cx,
            );
        }
        div()
            .relative()
            .size_full()
            .debug_selector(|| "helm-release-history".into())
            .child(
                uniform_list(
                    "helm-release-history-list",
                    detail.history.revisions.len(),
                    cx.processor(|this, range, _, cx| this.revision_rows(range, cx)),
                )
                .track_scroll(&detail.history.scroll)
                .size_full(),
            )
            .child(Scrollbar::vertical(&detail.history.scroll))
            .into_any_element()
    }

    pub(super) fn revision_rows(
        &mut self,
        range: Range<usize>,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let Some(detail) = &self.detail else {
            return Vec::new();
        };
        let now = Timestamp::now();
        range
            .filter_map(|ix| {
                let revision = detail.history.revisions.get(ix)?;
                let tone = tone(revision.status);
                let colour = tone_colour(tone, cx);
                let selector = format!("helm-release-revision:{}", revision.revision);
                let version = match &revision.app_version {
                    Some(app) => format!("{}  \u{b7}  app {app}", revision.chart),
                    None => revision.chart.to_string(),
                };
                Some(
                    h_flex()
                        .id(("helm-release-revision", ix))
                        .debug_selector(move || selector)
                        .h(u(px(ROW_HEIGHT)))
                        .px(u(tokens.spacing.xl))
                        .gap(u(tokens.spacing.lg))
                        .items_center()
                        .child(
                            div()
                                .w(u(px(40.)))
                                .flex_none()
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .child(revision.revision.to_string()),
                        )
                        .child(
                            h_flex()
                                .w(u(px(130.)))
                                .flex_none()
                                .gap(u(px(6.)))
                                .items_center()
                                .text_color(colour)
                                .child(Icon::new(tone_icon(tone)).size(u(px(12.))).color(colour))
                                .child(revision.status.as_str()),
                        )
                        .child(
                            div()
                                .w(u(px(260.)))
                                .flex_none()
                                .truncate()
                                .child(SharedString::from(version)),
                        )
                        .child(
                            div()
                                .w(u(px(60.)))
                                .flex_none()
                                .text_color(colors.text_muted)
                                .child(age(revision.updated, now)),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_color(colors.text_muted)
                                .child(SharedString::from(revision.description.clone())),
                        )
                        .into_any_element(),
                )
            })
            .collect()
    }
}
