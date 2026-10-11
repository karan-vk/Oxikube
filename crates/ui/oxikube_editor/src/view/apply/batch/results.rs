//! The results panel of a multi-document apply: under the buffer, one row per document with what
//! happened to it (created, configured, unchanged, failed with the server's message, skipped with
//! why), in buffer order. A click on a row selects its document in the buffer; a field the
//! server rejected carries a squiggle there. The rows are a `uniform_list`.

use std::ops::Range;

use gpui::{
    AnyElement, Context, Hsla, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, UniformListScrollHandle, Window, div, px,
    uniform_list,
};
use oxikube_app::{BatchReport, DocumentResult, DocumentStatus};
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::layout::{h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Sizable as _, u};

use super::BatchSession;
use super::row::document_row;
use crate::view::ManifestEditor;

/// A row's height, before the UI zoom.
const ROW_HEIGHT: f32 = 22.;
/// The header's height, before the UI zoom.
const HEADER_HEIGHT: f32 = 28.;
/// Most rows shown before the list scrolls.
const VISIBLE_ROWS: f32 = 6.;

/// The results of the last multi-document apply of an editor.
pub(in crate::view) struct ResultsPanel {
    report: BatchReport,
    session: BatchSession,
    scroll: UniformListScrollHandle,
}

impl ResultsPanel {
    pub(super) fn new(report: BatchReport, session: BatchSession) -> Self {
        Self {
            report,
            session,
            scroll: UniformListScrollHandle::new(),
        }
    }
}

impl ManifestEditor {
    /// The results of the last multi-document apply, while the panel is open.
    pub fn apply_results(&self) -> Option<&BatchReport> {
        self.apply.results.as_ref().map(|panel| &panel.report)
    }

    /// Closes the results panel.
    pub fn close_apply_results(&mut self, cx: &mut Context<Self>) {
        if self.apply.results.take().is_some() {
            cx.notify();
        }
    }

    /// Selects document `index` of the last apply in the buffer, unless the buffer changed since
    /// (then its ranges mean nothing). Returns whether it did.
    pub fn show_document(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(panel) = self.apply.results.as_ref() else {
            return false;
        };
        let range = panel.session.ranges.get(index).cloned();
        let current = self.editor.read(cx).version() == panel.session.version;
        let Some(range) = range.filter(|_| current) else {
            return false;
        };
        self.editor.update(cx, |editor, cx| {
            use oxikube_ui::editor::EditorApi as _;
            editor.api(window, cx).select(range.start..range.start);
        });
        self.focus_buffer(window, cx);
        true
    }

    /// The results panel, when open.
    pub(in crate::view) fn results_panel(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let panel = self.apply.results.as_ref()?;
        let tokens = cx.tokens();
        let count = panel.report.documents.len();
        let height = HEADER_HEIGHT + ROW_HEIGHT * VISIBLE_ROWS.min(count.max(1) as f32);
        let summary = panel.report.summary();
        let scroll = panel.scroll.clone();
        let stale = self.editor.read(cx).version() != panel.session.version;
        let note = if stale {
            "The buffer changed since: rows no longer point into it."
        } else {
            "Applied in buffer order; a failure does not stop the rest, as with kubectl apply."
        };
        Some(
            v_flex()
                .id("manifest-apply-results")
                .debug_selector(|| "manifest-apply-results".into())
                .flex_none()
                .h(u(px(height)))
                .border_t_1()
                .border_color(tokens.colors.border)
                .bg(tokens.colors.surface)
                .child(
                    h_flex()
                        .flex_none()
                        .h(u(px(HEADER_HEIGHT)))
                        .gap(u(tokens.spacing.sm))
                        .px(u(tokens.spacing.md))
                        .items_center()
                        .border_b_1()
                        .border_color(tokens.colors.border_variant)
                        .text_size(u(tokens.font.small))
                        .child(div().text_color(tokens.colors.text).child("Apply results"))
                        .child(
                            div()
                                .debug_selector(|| "manifest-apply-summary".into())
                                .text_color(tokens.colors.text_muted)
                                .child(summary),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_color(tokens.colors.text_muted)
                                .child(note),
                        )
                        .child(
                            Button::new("manifest-apply-results-close")
                                .label("Close")
                                .ghost()
                                .xsmall()
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.close_apply_results(cx);
                                })),
                        ),
                )
                .child(
                    div().flex_1().min_h_0().child(
                        uniform_list(
                            "manifest-apply-result-rows",
                            count,
                            cx.processor(|view, range: Range<usize>, _, cx| {
                                view.result_rows(range, cx)
                            }),
                        )
                        .track_scroll(&scroll)
                        .size_full(),
                    ),
                )
                .into_any_element(),
        )
    }

    fn result_rows(&self, range: Range<usize>, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let Some(panel) = self.apply.results.as_ref() else {
            return Vec::new();
        };
        let tokens = cx.tokens();
        range
            .filter_map(|ix| Some((ix, panel.report.documents.get(ix)?)))
            .map(|(ix, result)| {
                let chip = (result.status.word(), tone(&result.status, &tokens.colors));
                document_row(
                    "manifest-apply-result",
                    ix,
                    ROW_HEIGHT,
                    chip,
                    &result.label,
                    message(result),
                    &tokens,
                )
                .on_click(cx.listener(move |view, _, window, cx| {
                    view.show_document(ix, window, cx);
                }))
                .into_any_element()
            })
            .collect()
    }
}

/// The colour of a row's word.
fn tone(status: &DocumentStatus, colors: &oxikube_ui::Colors) -> Hsla {
    match status {
        DocumentStatus::Created => colors.success,
        DocumentStatus::Configured => colors.accent,
        DocumentStatus::Unchanged => colors.text_muted,
        DocumentStatus::Failed(_) => colors.error,
        DocumentStatus::Skipped(_) => colors.warning,
    }
}

/// The text after a row's name: why it failed or was skipped, or that a fallback sent it.
fn message(result: &DocumentResult) -> String {
    match &result.status {
        DocumentStatus::Failed(_) | DocumentStatus::Skipped(_) => result.message.clone(),
        _ => match result.method {
            Some(method) if method.is_fallback() => {
                format!(
                    "the server cannot server-side apply it: sent as a {}",
                    method.as_str()
                )
            }
            _ => String::new(),
        },
    }
}

#[cfg(test)]
mod tests {
    use oxikube_app::{ApplyError, ApplyMethod};
    use oxikube_ui::Tokens;

    use super::*;

    fn result(status: DocumentStatus, method: Option<ApplyMethod>, text: &str) -> DocumentResult {
        DocumentResult {
            index: 0,
            label: "ConfigMap default/a".to_owned(),
            target: None,
            status,
            method,
            message: text.to_owned(),
        }
    }

    #[test]
    fn each_status_has_its_own_colour_and_word() {
        let colors = Tokens::dark().colors;
        let failed = DocumentStatus::Failed(ApplyError::Invalid {
            message: "no".into(),
            causes: Vec::new(),
        });
        let rows = [
            (DocumentStatus::Created, colors.success, "created"),
            (DocumentStatus::Configured, colors.accent, "configured"),
            (DocumentStatus::Unchanged, colors.text_muted, "unchanged"),
            (failed, colors.error, "failed"),
            (
                DocumentStatus::Skipped("why".into()),
                colors.warning,
                "skipped",
            ),
        ];
        for (status, colour, word) in rows {
            assert_eq!(status.word(), word);
            assert_eq!(tone(&status, &colors), colour, "{word}");
        }
    }

    #[test]
    fn a_failure_and_a_skip_show_their_text_and_a_fallback_says_so() {
        let failed = DocumentStatus::Failed(ApplyError::Invalid {
            message: "ConfigMap \"a\" is invalid".into(),
            causes: Vec::new(),
        });
        let line = result(failed, None, "ConfigMap \"a\" is invalid");
        assert_eq!(message(&line), "ConfigMap \"a\" is invalid");
        let skipped = result(
            DocumentStatus::Skipped("skipped: it needs document 1".into()),
            None,
            "skipped: it needs document 1",
        );
        assert_eq!(message(&skipped), "skipped: it needs document 1");

        let plain = result(DocumentStatus::Created, Some(ApplyMethod::ServerSide), "x");
        assert_eq!(message(&plain), "", "nothing to add for a normal apply");
        let fallback = result(DocumentStatus::Created, Some(ApplyMethod::Replace), "x");
        assert!(
            message(&fallback).contains("sent as a"),
            "{}",
            message(&fallback)
        );
    }
}
