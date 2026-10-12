//! Drawing the Used by tab: a summary and its notes above the workload rows.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, div, list, px,
};
use oxikube_app::{UsedBy, WorkloadUse};
use oxikube_ui::layout::{StyledExt as _, h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Tokens, u};

use crate::detail::DetailView;
use crate::detail::parts::{empty, full_width, skeleton};

impl DetailView {
    /// The Used by tab: a skeleton until the first answer, a note while the Pods are read or when
    /// they cannot be, else the summary and the workloads.
    pub(in crate::detail) fn used_by_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let kind = self.ref_kind().map_or("object", |k| k.title());
        let note = |selector: &'static str, text: String| {
            div()
                .debug_selector(move || selector.to_owned())
                .p(u(tokens.spacing.xl))
                .text_color(tokens.colors.text_muted)
                .child(text)
                .into_any_element()
        };
        match &self.used_by.answer {
            None => skeleton(&tokens, "detail-used-by-loading", &[300., 240., 280.]),
            Some(UsedBy::Warming) => note("detail-used-by-warming", "Reading the pods…".to_owned()),
            Some(UsedBy::Unavailable(reason)) => note(
                "detail-used-by-unavailable",
                format!("The pods cannot be read, so what uses this {kind} is not known: {reason}"),
            ),
            Some(UsedBy::OutsideScope) => note(
                "detail-used-by-outside",
                format!(
                    "The pods of {} are not read (outside the selected namespaces), so what uses this {kind} is not known.",
                    self.target.namespace().unwrap_or_default()
                ),
            ),
            Some(UsedBy::Known {
                workloads,
                pods,
                partial,
            }) => {
                let summary = if workloads.is_empty() {
                    format!("No running pod uses this {kind}.")
                } else {
                    format!(
                        "Used by {} · {}",
                        plural(workloads.len(), "workload"),
                        plural(*pods, "pod")
                    )
                };
                v_flex()
                    .size_full()
                    .child(self.used_by_header(&tokens, summary, *partial))
                    .when(!workloads.is_empty(), |body| {
                        body.child(
                            div()
                                .debug_selector(|| "detail-used-by".to_owned())
                                .flex_1()
                                .min_h_0()
                                .child(
                                    list(
                                        self.used_by.list.clone(),
                                        cx.processor(|this, ix, _, cx| {
                                            full_width(this.used_by_row(ix, cx))
                                        }),
                                    )
                                    .size_full(),
                                ),
                        )
                    })
                    .into_any_element()
            }
        }
    }

    /// The summary line and what it leaves out.
    fn used_by_header(&self, tokens: &Tokens, summary: String, partial: bool) -> AnyElement {
        let colors = tokens.colors;
        let muted = |selector: &'static str, text: &str| {
            div()
                .debug_selector(move || selector.to_owned())
                .text_size(u(tokens.font.small))
                .text_color(colors.text_muted)
                .child(text.to_owned())
        };
        v_flex()
            .flex_none()
            .gap(u(px(2.)))
            .px(u(tokens.spacing.lg))
            .py(u(tokens.spacing.md))
            .border_b_1()
            .border_color(colors.border_variant)
            .child(
                div()
                    .debug_selector(|| "detail-used-by-summary".to_owned())
                    .font_semibold()
                    .child(summary),
            )
            .child(muted(
                "detail-used-by-basis",
                "From the specs of the running pods: a workload scaled to zero is not listed.",
            ))
            .when(partial, |header| {
                header.child(muted(
                    "detail-used-by-partial",
                    "Only the pods of the selected namespaces are counted; pods elsewhere may use it too.",
                ))
            })
            .into_any_element()
    }

    /// One workload. Only rows on screen are built.
    fn used_by_row(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some(UsedBy::Known { workloads, .. }) = &self.used_by.answer else {
            return empty();
        };
        let Some(WorkloadUse { target, pods }) = workloads.get(ix) else {
            return empty();
        };
        let tokens = cx.tokens();
        let colors = tokens.colors;
        // The workloads of a namespaced object are in its namespace; a PriorityClass's are
        // everywhere, so each says where it is.
        let namespace = self
            .ref_kind()
            .is_some_and(|kind| kind.is_cluster_scoped())
            .then(|| target.namespace.as_deref().map(str::to_owned))
            .flatten();
        let (open, kind, name) = (
            target.clone(),
            target.gvk.kind.to_string(),
            target.name.to_string(),
        );
        let pods = *pods;
        h_flex()
            .id(("used-by-row", ix))
            .debug_selector(move || format!("used-by-row-{ix}"))
            .gap(u(tokens.spacing.md))
            .items_center()
            .px(u(tokens.spacing.lg))
            .py(u(tokens.spacing.sm))
            .border_b_1()
            .border_color(colors.border_variant)
            .cursor_pointer()
            .hover(|style| style.bg(colors.element))
            .on_click(cx.listener(move |this, _, _, cx| this.open_resource(open.clone(), cx)))
            .child(
                div()
                    .flex_none()
                    .px(u(tokens.spacing.md))
                    .rounded(u(tokens.radius.sm))
                    .bg(colors.element)
                    .text_size(u(tokens.font.small))
                    .text_color(colors.text_muted)
                    .child(kind),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(colors.accent)
                    .child(name),
            )
            .children(namespace.map(|ns| {
                div()
                    .flex_none()
                    .text_size(u(tokens.font.small))
                    .text_color(colors.text_muted)
                    .child(ns)
            }))
            .child(
                div()
                    .flex_none()
                    .text_size(u(tokens.font.small))
                    .text_color(colors.text_muted)
                    .child(plural(pods, "pod")),
            )
            .into_any_element()
    }
}

fn plural(n: usize, word: &str) -> String {
    if n == 1 {
        format!("1 {word}")
    } else {
        format!("{n} {word}s")
    }
}
