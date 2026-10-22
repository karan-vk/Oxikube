//! The drawing of upgrade, rollback and uninstall: the header's actions and the History tab's
//! preview.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    Styled as _, div, px,
};
use oxikube_app::helm_ops::DiffKind;
use oxikube_domain::helm::HelmFeature;
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::checkbox::Checkbox;
use oxikube_ui::layout::{h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Sizable as _, u};

use super::change::Preview;
use super::detail::ReleaseDetail;
use super::view::HelmReleasesView;

/// The diff shows at most this many lines; a long one says how many more there are.
const MAX_DIFF_LINES: usize = 14;

impl HelmReleasesView {
    /// The header's actions: upgrade, and uninstall with its keep-history option, each drawn only
    /// when the gate allows it (a rollback is on the History tab).
    pub(super) fn header_actions(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let upgrade = self.offers(HelmFeature::Upgrade, cx);
        let uninstall = self.offers(HelmFeature::Uninstall, cx);
        let keep = self.keeps_history();
        h_flex()
            .debug_selector(|| "helm-release-actions".into())
            .gap(u(tokens.spacing.md))
            .items_center()
            .when(upgrade, |row| {
                row.child(
                    div()
                        .debug_selector(|| "helm-release-upgrade".into())
                        .child(
                            Button::new("helm-release-upgrade-button")
                                .small()
                                .label("Upgrade")
                                .on_click(cx.listener(|this, _, _, cx| this.upgrade(cx))),
                        ),
                )
            })
            .when(uninstall, |row| {
                row.child(
                    div()
                        .debug_selector(|| "helm-release-keep-history".into())
                        .child(
                            Checkbox::new("helm-release-keep-history-box")
                                .label("Keep history")
                                .checked(keep)
                                .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                    this.set_keep_history(*checked, cx)
                                })),
                        ),
                )
                .child(
                    div()
                        .debug_selector(|| "helm-release-uninstall".into())
                        .child(
                            Button::new("helm-release-uninstall-button")
                                .danger()
                                .small()
                                .label("Uninstall")
                                .on_click(cx.listener(|this, _, _, cx| this.uninstall(cx))),
                        ),
                )
            })
            .into_any_element()
    }

    /// The preview under the History list: the selected revision, its chart, what its values
    /// change, and the rollback button. `None` while no revision is selected.
    pub(super) fn rollback_pane(
        &self,
        detail: &ReleaseDetail,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let revision = detail.history.selected?;
        let target = detail
            .history
            .revisions
            .iter()
            .find(|r| r.revision == revision)?;
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let mut facts = format!("Chart {}", target.chart);
        if let Some(app) = &target.app_version {
            facts.push_str(&format!("  \u{b7}  app {app}"));
        }
        let body = match &detail.history.preview {
            Preview::None | Preview::Loading => self.skeleton("helm-release-preview-loading", cx),
            Preview::Failed(why) => self.note("helm-release-preview-failed", why.clone(), true, cx),
            Preview::Ready(lines) if lines.is_empty() => self.note(
                "helm-release-preview-same",
                "The values are the same as now.",
                false,
                cx,
            ),
            Preview::Ready(lines) => {
                let hidden = lines.len().saturating_sub(MAX_DIFF_LINES);
                v_flex()
                    .debug_selector(|| "helm-release-preview-diff".into())
                    .px(u(tokens.spacing.xl))
                    .font_family(cx.mono_font_family())
                    .text_size(u(tokens.font.small))
                    .children(lines.iter().take(MAX_DIFF_LINES).map(|line| {
                        let (mark, colour) = match line.kind {
                            DiffKind::Same => (' ', colors.text_muted),
                            DiffKind::Removed => ('-', colors.error),
                            DiffKind::Added => ('+', colors.success),
                        };
                        div()
                            .text_color(colour)
                            .child(SharedString::from(format!("{mark} {}", line.text)))
                    }))
                    .when(hidden > 0, |column| {
                        column.child(
                            div()
                                .text_color(colors.text_muted)
                                .child(format!("... {hidden} more lines")),
                        )
                    })
                    .into_any_element()
            }
        };
        Some(
            v_flex()
                .debug_selector(|| "helm-release-rollback-pane".into())
                .flex_none()
                .max_h(u(px(300.)))
                .overflow_hidden()
                .gap(u(tokens.spacing.sm))
                .py(u(tokens.spacing.md))
                .border_t_1()
                .border_color(colors.border_variant)
                .child(
                    h_flex()
                        .px(u(tokens.spacing.xl))
                        .gap(u(tokens.spacing.lg))
                        .items_center()
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .child(
                                    div()
                                        .debug_selector(|| "helm-release-preview-title".into())
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .child(format!(
                                            "Roll back to revision {revision} (now revision {})",
                                            detail.release.revision
                                        )),
                                )
                                .child(
                                    div()
                                        .debug_selector(|| "helm-release-preview-facts".into())
                                        .text_size(u(tokens.font.small))
                                        .text_color(colors.text_muted)
                                        .truncate()
                                        .child(SharedString::from(facts)),
                                ),
                        )
                        .child(
                            div()
                                .debug_selector(|| "helm-release-rollback-cancel".into())
                                .child(
                                    Button::new("helm-release-rollback-cancel-button")
                                        .ghost()
                                        .small()
                                        .label("Cancel")
                                        .on_click(
                                            cx.listener(|this, _, _, cx| this.clear_revision(cx)),
                                        ),
                                ),
                        )
                        .child(
                            div()
                                .debug_selector(|| "helm-release-rollback".into())
                                .child(
                                    Button::new("helm-release-rollback-button")
                                        .primary()
                                        .small()
                                        .label(format!("Roll back to revision {revision}"))
                                        .on_click(
                                            cx.listener(|this, _, _, cx| {
                                                this.rollback_selected(cx)
                                            }),
                                        ),
                                ),
                        ),
                )
                .child(body)
                .into_any_element(),
        )
    }
}
