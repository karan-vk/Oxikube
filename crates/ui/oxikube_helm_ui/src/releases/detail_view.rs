//! The detail pane: header, tabs and the shown tab's body.

use std::ops::Range;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, div, px, uniform_list,
};
use jiff::Timestamp;
use oxikube_domain::command::Command;
use oxikube_domain::helm::HelmFeature;
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::layout::{Scrollbar, h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Icon, IconName, Sizable as _, u};

use super::backend::{ObjectRow, ObjectState};
use super::delegate::{tone_colour, tone_icon};
use super::detail::ReleaseDetail;
use super::model::{Load, Tab, ValuesMode, age, tone};
use super::view::HelmReleasesView;

const ROW_HEIGHT: f32 = 28.0;

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

    fn note(
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
    fn skeleton(&self, id: &'static str, cx: &mut Context<Self>) -> AnyElement {
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

    // --- Resources --------------------------------------------------------------------------

    fn resources_body(&self, detail: &ReleaseDetail, cx: &mut Context<Self>) -> AnyElement {
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

    fn object_rows(&mut self, range: Range<usize>, cx: &mut Context<Self>) -> Vec<AnyElement> {
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

    // --- Values and Manifest -----------------------------------------------------------------

    fn reveal_button(&self, revealed: bool, cx: &mut Context<Self>) -> AnyElement {
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

    fn values_body(&self, detail: &ReleaseDetail, cx: &mut Context<Self>) -> AnyElement {
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

    fn manifest_body(&self, detail: &ReleaseDetail, cx: &mut Context<Self>) -> AnyElement {
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

    // --- Notes -------------------------------------------------------------------------------

    fn notes_body(&self, detail: &ReleaseDetail, cx: &mut Context<Self>) -> AnyElement {
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

    // --- History -----------------------------------------------------------------------------

    fn history_body(&self, detail: &ReleaseDetail, cx: &mut Context<Self>) -> AnyElement {
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

    fn revision_rows(&mut self, range: Range<usize>, cx: &mut Context<Self>) -> Vec<AnyElement> {
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
