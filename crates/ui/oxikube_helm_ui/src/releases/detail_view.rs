//! The detail pane: header, tabs and the shown tab's body.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    Styled as _, div, px,
};
use jiff::Timestamp;
use oxikube_domain::helm::HelmFeature;
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::layout::{h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Icon, IconName, Sizable as _, u};

use super::delegate::{tone_colour, tone_icon};
use super::detail::ReleaseDetail;
use super::model::{Load, Tab, age, tone};
use super::view::HelmReleasesView;

impl HelmReleasesView {
    /// The detail of the selected release, `None` when none is selected.
    pub(super) fn detail_pane(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let detail = self.detail.as_ref()?;
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let release = &detail.release;
        let tone = tone(release.status);
        let colour = tone_colour(tone, cx);
        let mut facts = format!(
            "Chart {}  \u{b7}  revision {}",
            release.chart, release.revision,
        );
        if let Some(app) = &release.app_version {
            facts.push_str(&format!("  \u{b7}  app {app}"));
        }
        facts.push_str(&format!(
            "  \u{b7}  updated {} ago",
            age(release.updated, Timestamp::now())
        ));
        // Mutating actions (rollback, uninstall; E16-S06) mount in `helm-release-actions`, and only
        // when the gate allows them. Without `helm` the view is read-only and says so.
        let gate = self.deps.gate.read(cx);
        let can_mutate = gate.allows(HelmFeature::Rollback) || gate.allows(HelmFeature::Uninstall);
        let tabs = Tab::ALL.into_iter().fold(
            h_flex().gap(u(tokens.spacing.sm)).px(u(tokens.spacing.xl)),
            |row, tab| {
                let button = Button::new(("helm-release-tab", tab as usize))
                    .small()
                    .label(tab.title())
                    .on_click(cx.listener(move |this, _, _, cx| this.set_tab(tab, cx)));
                row.child(
                    div()
                        .debug_selector(move || format!("helm-release-tab:{}", tab.key()))
                        .child(if tab == detail.tab {
                            button.primary()
                        } else {
                            button.ghost()
                        }),
                )
            },
        );
        Some(
            v_flex()
                .debug_selector(|| "helm-release-detail".into())
                .size_full()
                .child(
                    h_flex()
                        .flex_none()
                        .gap(u(tokens.spacing.lg))
                        .px(u(tokens.spacing.xl))
                        .py(u(tokens.spacing.md))
                        .items_center()
                        .child(Icon::new(tone_icon(tone)).size(u(px(16.))).color(colour))
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .child(
                                    div()
                                        .debug_selector(|| "helm-release-title".into())
                                        .text_size(u(tokens.font.heading))
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .truncate()
                                        .child(SharedString::from(release.release.to_string())),
                                )
                                .child(
                                    div()
                                        .debug_selector(|| "helm-release-facts".into())
                                        .text_size(u(tokens.font.small))
                                        .text_color(colors.text_muted)
                                        .truncate()
                                        .child(SharedString::from(facts)),
                                ),
                        )
                        .child(
                            div()
                                .debug_selector(|| "helm-release-status".into())
                                .text_color(colour)
                                .child(release.status.as_str()),
                        )
                        .when(can_mutate, |row| {
                            row.child(div().debug_selector(|| "helm-release-actions".into()))
                        })
                        .when(!can_mutate, |row| {
                            row.child(
                                div()
                                    .debug_selector(|| "helm-release-read-only".into())
                                    .text_size(u(tokens.font.small))
                                    .text_color(colors.text_muted)
                                    .child("Read-only: changing releases needs the helm binary"),
                            )
                        })
                        .child(
                            Button::new("helm-release-close")
                                .ghost()
                                .xsmall()
                                .icon(Icon::new(IconName::X).size(u(px(12.))))
                                .on_click(cx.listener(|this, _, _, cx| this.close_detail(cx))),
                        ),
                )
                .child(tabs)
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .mt(u(tokens.spacing.sm))
                        .border_t_1()
                        .border_color(colors.border_variant)
                        .child(self.tab_body(detail, cx)),
                )
                .into_any_element(),
        )
    }

    pub(super) fn note(
        &self,
        id: &'static str,
        text: impl Into<SharedString>,
        error: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tokens = cx.tokens();
        div()
            .debug_selector(move || id.into())
            .p(u(tokens.spacing.xl))
            .text_color(if error {
                tokens.colors.error
            } else {
                tokens.colors.text_muted
            })
            .child(text.into())
            .into_any_element()
    }

    /// Placeholder bars while a tab reads: the detail is on screen at once, the data follows.
    pub(super) fn skeleton(&self, id: &'static str, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        v_flex()
            .debug_selector(move || id.into())
            .gap(u(tokens.spacing.md))
            .p(u(tokens.spacing.xl))
            .children([0.9, 0.7, 0.8, 0.5, 0.6].map(|share| {
                div()
                    .h(u(px(12.)))
                    .w(gpui::relative(share))
                    .rounded(u(px(3.)))
                    .bg(tokens.colors.element_hover)
            }))
            .into_any_element()
    }

    fn tab_body(&self, detail: &ReleaseDetail, cx: &mut Context<Self>) -> AnyElement {
        match (detail.tab, detail.load(detail.tab)) {
            (Tab::Resources, Load::Idle | Load::Loading) => {
                self.skeleton("helm-release-resources-loading", cx)
            }
            (Tab::Values, Load::Idle | Load::Loading) => {
                self.skeleton("helm-release-values-loading", cx)
            }
            (Tab::Manifest, Load::Idle | Load::Loading) => {
                self.skeleton("helm-release-manifest-loading", cx)
            }
            (Tab::Notes, Load::Idle | Load::Loading) => {
                self.skeleton("helm-release-notes-loading", cx)
            }
            (Tab::History, Load::Idle | Load::Loading) => {
                self.skeleton("helm-release-history-loading", cx)
            }
            (tab, Load::Failed(why)) => {
                let id = match tab {
                    Tab::Resources => "helm-release-resources-failed",
                    Tab::Values => "helm-release-values-failed",
                    Tab::Manifest => "helm-release-manifest-failed",
                    Tab::Notes => "helm-release-notes-failed",
                    Tab::History => "helm-release-history-failed",
                };
                self.note(id, why.clone(), true, cx)
            }
            (Tab::Resources, Load::Ready) => self.resources_body(detail, cx),
            (Tab::Values, Load::Ready) => self.values_body(detail, cx),
            (Tab::Manifest, Load::Ready) => self.manifest_body(detail, cx),
            (Tab::Notes, Load::Ready) => self.notes_body(detail, cx),
            (Tab::History, Load::Ready) => self.history_body(detail, cx),
        }
    }
}
