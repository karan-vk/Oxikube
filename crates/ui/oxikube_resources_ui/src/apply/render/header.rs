//! The header: the title, the cluster and its lock, the stages, and the source bar with the file
//! pickers and the namespace.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement as _, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, div, px,
};
use oxikube_ui::button::Button;
use oxikube_ui::input::Input;
use oxikube_ui::layout::{Disableable as _, h_flex};
use oxikube_ui::{ActiveTokens as _, Icon, IconName, Sizable as _, u};

use crate::apply::model::Stage;
use crate::apply::source::SourceKind;
use crate::apply::view::ApplyView;

impl ApplyView {
    pub(super) fn header(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let busy = self.model.busy();
        let source = self
            .source
            .as_ref()
            .map_or_else(|| "No file or directory chosen".to_owned(), |s| s.display());
        let scoped = self
            .model
            .rows()
            .iter()
            .filter(|row| row.namespace.is_none())
            .count();
        gpui::div()
            .flex_none()
            .child(
                h_flex()
                    .gap(u(tokens.spacing.lg))
                    .px(u(tokens.spacing.xl))
                    .py(u(tokens.spacing.md))
                    .items_center()
                    .border_b_1()
                    .border_color(colors.border_variant)
                    .child(
                        div()
                            .text_size(u(tokens.font.heading))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child("Apply"),
                    )
                    .child(
                        div()
                            .debug_selector(|| "apply-cluster".into())
                            .text_color(colors.text_muted)
                            .child(SharedString::from(format!("to {}", self.cluster_label))),
                    )
                    .when(self.read_only, |bar| {
                        bar.child(
                            h_flex()
                                .debug_selector(|| "apply-lock".into())
                                .gap(u(tokens.spacing.xs))
                                .items_center()
                                .text_color(colors.warning)
                                .child(Icon::new(IconName::Lock).size(u(px(14.))))
                                .child("read-only"),
                        )
                    })
                    .when(scoped > 0, |bar| {
                        bar.child(
                            div()
                                .debug_selector(|| "apply-cluster-scoped".into())
                                .text_color(colors.warning)
                                .child(SharedString::from(format!(
                                    "{scoped} cluster-scoped, not in a namespace"
                                ))),
                        )
                    })
                    .child(div().flex_1())
                    .children(self.stage_chips(cx)),
            )
            .child(
                h_flex()
                    .gap(u(tokens.spacing.md))
                    .px(u(tokens.spacing.xl))
                    .py(u(tokens.spacing.md))
                    .items_center()
                    .border_b_1()
                    .border_color(colors.border_variant)
                    .child(
                        div()
                            .debug_selector(|| "apply-source".into())
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_color(if self.source.is_some() {
                                colors.text
                            } else {
                                colors.text_muted
                            })
                            .child(SharedString::from(source)),
                    )
                    .child(
                        div().debug_selector(|| "apply-pick-files".into()).child(
                            Button::new("apply-pick-files-button")
                                .small()
                                .outline()
                                .icon(Icon::new(IconName::FolderOpen).size(u(px(14.))))
                                .label("File or folder")
                                .disabled(busy)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.pick(SourceKind::Manifests, cx)
                                })),
                        ),
                    )
                    .when(self.kustomize.is_some(), |bar| {
                        bar.child(
                            div()
                                .debug_selector(|| "apply-pick-kustomize".into())
                                .child(
                                    Button::new("apply-pick-kustomize-button")
                                        .small()
                                        .outline()
                                        .icon(Icon::new(IconName::Layers).size(u(px(14.))))
                                        .label("Kustomize folder")
                                        .tooltip(
                                            self.kustomize
                                                .as_ref()
                                                .map(ToString::to_string)
                                                .unwrap_or_default(),
                                        )
                                        .disabled(busy)
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.pick(SourceKind::Kustomize, cx)
                                        })),
                                ),
                        )
                    })
                    .child(div().text_color(colors.text_muted).child("Namespace"))
                    .child(
                        div()
                            .debug_selector(|| "apply-namespace".into())
                            .w(u(px(160.)))
                            .child(Input::new(&self.namespace).small()),
                    ),
            )
            .into_any_element()
    }

    /// The Load, Review and Result chips: the shown one is lit, the others switch to it.
    fn stage_chips(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        [
            (Stage::Load, "Load", "apply-stage-load"),
            (Stage::Review, "Review", "apply-stage-review"),
            (Stage::Result, "Result", "apply-stage-result"),
        ]
        .into_iter()
        .map(|(stage, label, id)| {
            let shown = self.model.stage() == stage;
            let available = match stage {
                Stage::Load => true,
                Stage::Review | Stage::Result => !self.model.rows().is_empty(),
            };
            div()
                .id(id)
                .debug_selector(move || id.into())
                .px(u(tokens.spacing.md))
                .py(u(tokens.spacing.xs))
                .rounded(u(tokens.radius.sm))
                .text_color(match (shown, available) {
                    (true, _) => colors.on_accent,
                    (false, true) => colors.text,
                    (false, false) => colors.text_disabled,
                })
                .when(shown, |chip| chip.bg(colors.accent))
                .when(available && !shown, |chip| {
                    chip.cursor_pointer()
                        .hover(|chip| chip.bg(colors.element_hover))
                        .on_click(cx.listener(move |this, _, _, cx| this.show_stage(stage, cx)))
                })
                .child(label)
                .into_any_element()
        })
        .collect()
    }
}
