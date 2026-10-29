//! The Load view: what was read (files, documents, the kustomize tool and its stderr), the parse
//! errors with file and line, and the way to start when nothing is loaded yet.

use std::ops::Range;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement as _, ParentElement as _,
    SharedString, Styled as _, div, px, uniform_list,
};
use oxikube_ui::button::Button;
use oxikube_ui::layout::{h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Icon, IconName, Sizable as _, u};

use super::ROW_HEIGHT;
use crate::apply::source::ApplySource;
use crate::apply::view::ApplyView;

impl ApplyView {
    pub(super) fn load_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        if let Some(problem) = self
            .problem
            .clone()
            .filter(|_| self.model.summary().is_none())
        {
            return self.problem_card(problem, cx);
        }
        let Some(summary) = self.model.summary().cloned() else {
            return self.empty(cx);
        };
        let count = summary.errors.len();
        let scroll = self.errors_scroll.clone();
        let line = format!(
            "{} {} from {} {}",
            summary.documents,
            if summary.documents == 1 {
                "document"
            } else {
                "documents"
            },
            summary.files,
            if summary.files == 1 { "file" } else { "files" },
        );
        v_flex()
            .size_full()
            .p(u(tokens.spacing.xl))
            .gap(u(tokens.spacing.md))
            .child(
                div()
                    .debug_selector(|| "apply-load-summary".into())
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .child(SharedString::from(line)),
            )
            .when_some(summary.tool.clone(), |view, tool| {
                view.child(
                    div()
                        .debug_selector(|| "apply-kustomize-tool".into())
                        .text_color(colors.text_muted)
                        .child(SharedString::from(format!("Built with {tool}"))),
                )
            })
            .when(!summary.stderr.trim().is_empty(), |view| {
                view.child(
                    div()
                        .debug_selector(|| "apply-kustomize-stderr".into())
                        .font_family("monospace")
                        .text_color(colors.warning)
                        .child(SharedString::from(summary.stderr.clone())),
                )
            })
            .children(summary.notes.iter().take(5).map(|note| {
                div()
                    .text_color(colors.text_muted)
                    .child(SharedString::from(note.clone()))
            }))
            .when(self.docs.is_empty() && count == 0, |view| {
                view.child(
                    div()
                        .debug_selector(|| "apply-no-documents".into())
                        .text_color(colors.warning)
                        .child("No Kubernetes objects were found there."),
                )
            })
            .when(count > 0, |view| {
                view.child(
                    div()
                        .debug_selector(|| "apply-load-errors-title".into())
                        .text_color(colors.error)
                        .child(SharedString::from(format!(
                            "{count} {} could not be read; the rest still apply",
                            if count == 1 { "document" } else { "documents" }
                        ))),
                )
                .child(
                    div().flex_1().min_h_0().child(
                        uniform_list(
                            "apply-load-errors",
                            count,
                            cx.processor(|this, range: Range<usize>, _, cx| {
                                this.error_rows(range, cx)
                            }),
                        )
                        .track_scroll(&scroll)
                        .size_full(),
                    ),
                )
            })
            .into_any_element()
    }

    fn error_rows(&self, range: Range<usize>, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let tokens = cx.tokens();
        let Some(summary) = self.model.summary() else {
            return Vec::new();
        };
        range
            .filter_map(|ix| Some((ix, summary.errors.get(ix)?)))
            .map(|(ix, error)| {
                h_flex()
                    .id(("apply-load-error", ix))
                    .debug_selector(move || format!("apply-load-error-{ix}"))
                    .h(u(px(ROW_HEIGHT)))
                    .w_full()
                    .gap(u(tokens.spacing.md))
                    .items_center()
                    .child(
                        Icon::new(IconName::CircleX)
                            .size(u(px(14.)))
                            .color(tokens.colors.error),
                    )
                    .child(SharedString::from(error.to_string()))
                    .into_any_element()
            })
            .collect()
    }

    fn problem_card(&self, problem: String, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        v_flex()
            .size_full()
            .p(u(tokens.spacing.xl))
            .child(
                v_flex()
                    .debug_selector(|| "apply-problem".into())
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
                            .text_color(colors.error)
                            .child(Icon::new(IconName::CircleAlert).size(u(px(16.))))
                            .child(
                                div()
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .child("Could not load"),
                            ),
                    )
                    .child(
                        div()
                            .debug_selector(|| "apply-problem-text".into())
                            .font_family("monospace")
                            .text_color(colors.text_muted)
                            .child(SharedString::from(problem)),
                    ),
            )
            .into_any_element()
    }

    fn empty(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let last = self.last_dir.clone();
        v_flex()
            .debug_selector(|| "apply-empty".into())
            .size_full()
            .p(u(tokens.spacing.xl))
            .gap(u(tokens.spacing.md))
            .child(
                div()
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .child("Apply a file or a directory of manifests"),
            )
            .child(div().text_color(colors.text_muted).child(
                "Choose a manifest file, a folder (read recursively), or a kustomize folder. \
                 Each object is checked with the server's dry run before anything is applied.",
            ))
            .when_some(last, |view, last| {
                let path = last.clone();
                view.child(
                    h_flex().child(
                        div().debug_selector(|| "apply-last-dir".into()).child(
                            Button::new("apply-last-dir-button")
                                .small()
                                .outline()
                                .icon(Icon::new(IconName::Clock).size(u(px(14.))))
                                .label(format!("Last: {}", last.display()))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.load(ApplySource::manifests(path.clone()), cx)
                                })),
                        ),
                    ),
                )
            })
            .into_any_element()
    }
}
