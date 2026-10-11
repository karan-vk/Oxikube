//! The document list and the selected document's diff of the [`BatchPreviewDialog`].

use std::ops::Range;

use gpui::{
    AnyElement, Context, Hsla, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, div, prelude::FluentBuilder as _, px,
    uniform_list,
};
use oxikube_app::{Change, DocumentPreview, PreviewOutcome};
use oxikube_ui::{ActiveTokens as _, u};

use super::{BatchPreviewDialog, ROW_HEIGHT, VISIBLE_DIFF_ROWS, VISIBLE_ROWS};
use crate::view::apply::batch::row::document_row;
use crate::view::live::{ROW_HEIGHT as DIFF_ROW_HEIGHT, inline_rows};

impl BatchPreviewDialog {
    pub(super) fn list(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let tokens = cx.tokens();
        let count = self.preview.documents.len();
        let height = u(px(ROW_HEIGHT * VISIBLE_ROWS.min(count.max(1) as f32)));
        div()
            .debug_selector(|| "batch-preview-documents".into())
            .h(height)
            .w_full()
            .border_1()
            .border_color(tokens.colors.border_variant)
            .rounded(u(tokens.radius.sm))
            .overflow_hidden()
            .child(
                uniform_list(
                    "batch-preview-rows",
                    count,
                    cx.processor(|this, range: Range<usize>, _, cx| this.document_rows(range, cx)),
                )
                .track_scroll(&self.list_scroll)
                .size_full(),
            )
    }

    fn document_rows(&self, range: Range<usize>, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let tokens = cx.tokens();
        range
            .filter_map(|ix| Some((ix, self.preview.documents.get(ix)?)))
            .map(|(ix, document)| {
                let chip = chip(document, &tokens.colors);
                document_row(
                    "batch-preview-row",
                    ix,
                    ROW_HEIGHT,
                    chip,
                    &document.label,
                    detail(document),
                    &tokens,
                )
                .when(ix == self.selected, |row| {
                    row.bg(tokens.colors.element_selected)
                })
                .on_click(cx.listener(move |this, _, _, cx| this.select(ix, cx)))
                .into_any_element()
            })
            .collect()
    }

    /// The selected document's diff, or why there is none.
    pub(super) fn body(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let message = |text: String| {
            div()
                .debug_selector(|| "batch-preview-message".into())
                .p(u(tokens.spacing.md))
                .text_size(u(tokens.font.small))
                .text_color(tokens.colors.text_muted)
                .child(text)
                .into_any_element()
        };
        let Some(document) = self.preview.documents.get(self.selected) else {
            return message("Nothing to apply".into());
        };
        let diff = self.diffs.get(self.selected).cloned().flatten();
        let (Some(diff), PreviewOutcome::Ready(_)) = (diff, &document.outcome) else {
            return message(detail(document));
        };
        let rows = diff.inline_rows().len();
        let height = u(px(
            DIFF_ROW_HEIGHT * VISIBLE_DIFF_ROWS.min(rows.max(1) as f32)
        ));
        div()
            .debug_selector(|| "batch-preview-diff".into())
            .h(height)
            .w_full()
            .border_1()
            .border_color(tokens.colors.border_variant)
            .rounded(u(tokens.radius.sm))
            .overflow_hidden()
            .font_family(cx.mono_font_family())
            .text_size(u(tokens.font.mono))
            .child(
                uniform_list(
                    "batch-preview-diff-rows",
                    rows,
                    cx.processor(move |_, range: Range<usize>, _, cx| {
                        inline_rows(&diff, range, cx)
                    }),
                )
                .track_scroll(&self.diff_scroll)
                .size_full(),
            )
            .into_any_element()
    }
}

/// The chip of a document: what the dry run says, and its colour.
pub(super) fn chip(
    document: &DocumentPreview,
    colors: &oxikube_ui::Colors,
) -> (&'static str, Hsla) {
    match &document.outcome {
        PreviewOutcome::Ready(preview) => match preview.change() {
            Change::Create => ("create", colors.success),
            Change::Update => ("update", colors.accent),
            Change::Unchanged => ("unchanged", colors.text_muted),
        },
        PreviewOutcome::Deferred => ("after #", colors.accent),
        PreviewOutcome::Failed(_) => ("refused", colors.error),
        PreviewOutcome::Skipped => ("skipped", colors.warning),
    }
}

/// The line about a document after its name.
pub(super) fn detail(document: &DocumentPreview) -> String {
    match &document.outcome {
        PreviewOutcome::Ready(preview) => match preview.change() {
            Change::Create => "the server dry run creates it".to_owned(),
            Change::Unchanged => "the server dry run changes nothing".to_owned(),
            Change::Update => "the server dry run changes it".to_owned(),
        },
        PreviewOutcome::Deferred => match document.depends_on {
            Some(earlier) => format!(
                "needs document {}: rehearsed and applied once that exists",
                earlier + 1
            ),
            None => "rehearsed and applied once what it needs exists".to_owned(),
        },
        PreviewOutcome::Failed(error) => format!("{error}. It is not sent."),
        PreviewOutcome::Skipped => match document.depends_on {
            Some(earlier) => format!(
                "needs document {}, which the dry run refused. It is not sent.",
                earlier + 1
            ),
            None => "not sent".to_owned(),
        },
    }
}
