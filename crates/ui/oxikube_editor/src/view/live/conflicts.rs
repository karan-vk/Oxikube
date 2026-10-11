//! The merge's conflicts in the bottom panel: per conflict, a row with a button per side, then
//! the buffer's and the server's lines side by side. A `uniform_list` over the merge's
//! [`ConflictRow`]s (laid out once, off the UI thread, when the merge was made): only the rows on
//! screen are built, so a conflict thousands of lines long costs what one screen of it costs.

use std::ops::Range;

use gpui::{
    AnyElement, Context, Hsla, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, Styled as _, div, prelude::FluentBuilder as _, px, uniform_list,
};
use oxikube_domain::command::Command;
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::layout::{Selectable as _, h_flex};
use oxikube_ui::{ActiveTokens as _, Sizable as _, u};

use crate::diff::{ConflictRow, MergeSide};
use crate::view::ManifestEditor;

/// A conflict row's height (a line, or the header's extra-small buttons), before the UI zoom.
const CONFLICT_ROW_HEIGHT: f32 = 22.;

impl ManifestEditor {
    /// The pending merge's conflicts, virtualised.
    pub(super) fn conflicts(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let rows = self
            .live
            .sync
            .pending_merge()
            .map_or(0, |merge| merge.conflict_rows().len());
        let family = cx.mono_font_family();
        div()
            .id("manifest-conflicts")
            .debug_selector(|| "manifest-conflicts".into())
            .size_full()
            .px(u(tokens.spacing.sm))
            .child(
                uniform_list(
                    "manifest-conflict-rows",
                    rows,
                    cx.processor(|view, range: Range<usize>, _, cx| view.conflict_rows(range, cx)),
                )
                .track_scroll(&self.live.panel.conflict_scroll)
                .size_full()
                .font_family(family)
                .text_size(u(tokens.font.mono)),
            )
            .into_any_element()
    }

    /// The conflict rows `range`; nothing else is built.
    fn conflict_rows(&mut self, range: Range<usize>, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let Some(merge) = self.live.sync.pending_merge() else {
            return Vec::new();
        };
        let colors = &cx.tokens().colors;
        let (text, accent, border) = (colors.text, colors.accent, colors.border_variant);
        let rows = merge.conflict_rows();
        let mut out = Vec::with_capacity(range.len());
        for row in &rows[range.start.min(rows.len())..range.end.min(rows.len())] {
            let element = match row {
                ConflictRow::Header { conflict, chunk } => {
                    let chosen = merge.chosen_at(*chunk);
                    let mut take = |side: MergeSide| self.take_button(*conflict, side, chosen, cx);
                    conflict_row()
                        .border_t_1()
                        .border_color(border)
                        .child(take(MergeSide::Mine))
                        .child(take(MergeSide::Server))
                        .into_any_element()
                }
                ConflictRow::Line { chunk, .. } => {
                    let chosen = merge.chosen_at(*chunk);
                    let half = |side: MergeSide| {
                        line_half(merge.conflict_line(row, side), chosen == Some(side), accent)
                    };
                    conflict_row()
                        .text_color(text)
                        .child(half(MergeSide::Mine))
                        .child(half(MergeSide::Server))
                        .into_any_element()
                }
            };
            out.push(element);
        }
        out
    }

    /// "Use mine" / "Use server" for conflict `conflict`, selected when it is the side picked.
    fn take_button(
        &self,
        conflict: usize,
        side: MergeSide,
        chosen: Option<MergeSide>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let number = u32::try_from(conflict).unwrap_or(u32::MAX);
        let (label, command) = match side {
            MergeSide::Mine => ("mine", Command::EditorTakeMine { conflict: number }),
            MergeSide::Server => ("server", Command::EditorTakeServer { conflict: number }),
        };
        let id = SharedString::from(format!("manifest-take-{label}-{conflict}"));
        div()
            .id(id.clone())
            .debug_selector({
                let id = id.clone();
                move || id.to_string()
            })
            .flex_1()
            .min_w_0()
            .child(
                Button::new(id)
                    .label(format!("Use {label}"))
                    .xsmall()
                    .ghost()
                    .selected(chosen == Some(side))
                    .on_click(self.sends(command, cx)),
            )
            .into_any_element()
    }
}

fn conflict_row() -> gpui::Div {
    h_flex()
        .w_full()
        .h(u(px(CONFLICT_ROW_HEIGHT)))
        .gap(u(px(8.)))
        .items_center()
        .overflow_hidden()
        .whitespace_nowrap()
}

/// One side's line of a conflict row, marked when its side is the one picked.
fn line_half(line: Option<&str>, picked: bool, accent: Hsla) -> AnyElement {
    div()
        .flex_1()
        .min_w_0()
        .h_full()
        .px(u(px(4.)))
        .overflow_hidden()
        .when(picked, |d| d.border_l_2().border_color(accent))
        .child(SharedString::from(line.unwrap_or_default().to_owned()))
        .into_any_element()
}
