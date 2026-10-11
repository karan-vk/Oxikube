//! The editor's bottom panel: the diff against the live object, side by side or inline, or the
//! merge's conflicts while a merge waits for sides. The diff rows are a `uniform_list`: only the
//! rows on screen are built, so a diff of a 5 MB object costs what one screen of it costs.

use std::ops::Range;
use std::sync::Arc;

use gpui::{
    AnyElement, Context, Hsla, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, Styled as _, div, prelude::FluentBuilder as _, px, uniform_list,
};
use oxikube_domain::command::Command;
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::layout::{Selectable as _, h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Sizable as _, u};

use super::{DiffLayout, PanelBody};
use crate::diff::{InlineRow, LineKind, ManifestDiff, SideRow, SyncState};
use crate::view::ManifestEditor;

/// The panel's height, before the UI zoom.
const PANEL_HEIGHT: f32 = 260.;
/// A diff row's height, before the UI zoom.
const ROW_HEIGHT: f32 = 18.;

/// "No changes", "1 change · −1 +1".
pub(crate) fn summary(diff: &ManifestDiff) -> String {
    if diff.is_empty() {
        return "No changes".to_owned();
    }
    let (removed, added) = diff.counts();
    let hunks = match diff.hunks() {
        1 => "1 change".to_owned(),
        n => format!("{n} changes"),
    };
    format!("{hunks} · −{removed} +{added}")
}

impl ManifestEditor {
    /// The bottom panel, when the diff is open or a merge waits for sides.
    pub(in crate::view) fn diff_panel(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let merging = self.live.sync.state() == SyncState::Merging;
        if !self.live.panel.open && !merging {
            return None;
        }
        let tokens = cx.tokens();
        let body = if merging {
            self.conflicts(cx)
        } else {
            self.diff_body(cx)
        };
        Some(
            v_flex()
                .id("manifest-diff")
                .debug_selector(|| "manifest-diff".into())
                .flex_none()
                .h(u(px(PANEL_HEIGHT)))
                .border_t_1()
                .border_color(tokens.colors.border)
                .bg(tokens.colors.surface)
                .child(self.panel_header(merging, cx))
                .child(div().flex_1().min_h_0().child(body))
                .into_any_element(),
        )
    }

    fn panel_header(&self, merging: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let target = self
            .live
            .target_label()
            .unwrap_or_else(|| "the live object".to_owned());
        let (title, detail) = if merging {
            let left = self.live.sync.pending_merge().map_or(0, |m| m.unresolved());
            (
                format!("Merge with {target}"),
                format!("{left} conflicts to settle"),
            )
        } else {
            let detail = match &self.live.panel.body {
                PanelBody::Diff(diff) => summary(diff),
                PanelBody::Loading => "Comparing…".to_owned(),
                PanelBody::Message(_) => String::new(),
            };
            (format!("Diff vs live · {target}"), detail)
        };
        let side_by_side = self.live.panel.layout == DiffLayout::SideBySide;
        h_flex()
            .flex_none()
            .gap(u(tokens.spacing.sm))
            .px(u(tokens.spacing.md))
            .py(u(tokens.spacing.xs))
            .items_center()
            .border_b_1()
            .border_color(tokens.colors.border_variant)
            .text_size(u(tokens.font.small))
            .child(div().text_color(tokens.colors.text).child(title))
            .child(
                div()
                    .id("manifest-diff-summary")
                    .debug_selector(|| "manifest-diff-summary".into())
                    .text_color(tokens.colors.text_muted)
                    .child(detail),
            )
            .child(div().flex_1())
            .when(!merging, |row| {
                row.child(
                    Button::new("manifest-diff-layout")
                        .label(if side_by_side {
                            "Side by side"
                        } else {
                            "Inline"
                        })
                        .ghost()
                        .xsmall()
                        .selected(side_by_side)
                        .on_click(cx.listener(|view, _, window, cx| {
                            view.focus_buffer(window, cx);
                            view.send(Command::EditorToggleDiffLayout, cx);
                        })),
                )
                .child(
                    Button::new("manifest-diff-close")
                        .label("Close")
                        .ghost()
                        .xsmall()
                        .on_click(cx.listener(|view, _, window, cx| {
                            view.focus_buffer(window, cx);
                            view.send(Command::EditorShowDiff, cx);
                        })),
                )
            })
    }

    fn diff_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let message = |text: SharedString| {
            div()
                .p(u(tokens.spacing.md))
                .text_size(u(tokens.font.small))
                .text_color(tokens.colors.text_muted)
                .child(text)
                .into_any_element()
        };
        let diff = match &self.live.panel.body {
            PanelBody::Loading => return message(self.loading_text()),
            PanelBody::Message(text) => return message(text.clone()),
            PanelBody::Diff(diff) if diff.is_empty() => {
                return message("The buffer matches the live object.".into());
            }
            PanelBody::Diff(diff) => diff.clone(),
        };
        let layout = self.live.panel.layout;
        let rows = match layout {
            DiffLayout::SideBySide => diff.side_rows().len(),
            DiffLayout::Inline => diff.inline_rows().len(),
        };
        let family = cx.mono_font_family();
        uniform_list(
            "manifest-diff-rows",
            rows,
            cx.processor(move |_, range: Range<usize>, _, cx| diff_rows(&diff, layout, range, cx)),
        )
        .track_scroll(&self.live.panel.scroll)
        .size_full()
        .font_family(family)
        .text_size(u(tokens.font.mono))
        .into_any_element()
    }

    fn loading_text(&self) -> SharedString {
        match &self.live.target {
            Some(_) => "Waiting for the live object…".into(),
            None => "Looking up the object the manifest names…".into(),
        }
    }
}

/// The rows `range` of the diff; nothing else is built.
fn diff_rows(
    diff: &Arc<ManifestDiff>,
    layout: DiffLayout,
    range: Range<usize>,
    cx: &mut Context<ManifestEditor>,
) -> Vec<AnyElement> {
    let colors = Colors::of(cx);
    range
        .map(|ix| match layout {
            DiffLayout::Inline => inline_row(diff, diff.inline_rows()[ix], &colors),
            DiffLayout::SideBySide => side_row(diff, diff.side_rows()[ix], &colors),
        })
        .collect()
}

struct Colors {
    text: Hsla,
    muted: Hsla,
    added: Hsla,
    removed: Hsla,
}

impl Colors {
    fn of(cx: &Context<ManifestEditor>) -> Self {
        let colors = &cx.tokens().colors;
        Self {
            text: colors.text,
            muted: colors.text_muted,
            added: colors.success.opacity(0.18),
            removed: colors.error.opacity(0.18),
        }
    }

    fn background(&self, kind: LineKind) -> Option<Hsla> {
        match kind {
            LineKind::Same => None,
            LineKind::Added => Some(self.added),
            LineKind::Removed => Some(self.removed),
        }
    }
}

fn row() -> gpui::Div {
    h_flex()
        .w_full()
        .h(u(px(ROW_HEIGHT)))
        .overflow_hidden()
        .whitespace_nowrap()
}

fn gutter(number: Option<u32>, colors: &Colors) -> impl IntoElement {
    div()
        .flex_none()
        .w(u(px(40.)))
        .pr(u(px(6.)))
        .text_right()
        .text_color(colors.muted)
        .child(number.map_or_else(String::new, |n| (n + 1).to_string()))
}

fn sign(kind: LineKind) -> &'static str {
    match kind {
        LineKind::Same => " ",
        LineKind::Added => "+",
        LineKind::Removed => "−",
    }
}

fn gap(lines: u32, colors: &Colors) -> AnyElement {
    let text = match lines {
        1 => "⋯ 1 unchanged line".to_owned(),
        n => format!("⋯ {n} unchanged lines"),
    };
    row()
        .pl(u(px(46.)))
        .text_color(colors.muted)
        .child(text)
        .into_any_element()
}

fn inline_row(diff: &ManifestDiff, row_data: InlineRow, colors: &Colors) -> AnyElement {
    let (kind, old, new) = match row_data {
        InlineRow::Line { kind, old, new } => (kind, old, new),
        InlineRow::Gap(lines) => return gap(lines, colors),
    };
    let text = match (new, old) {
        (Some(n), _) => diff.new_line(n),
        (None, Some(o)) => diff.old_line(o),
        (None, None) => "",
    };
    row()
        .when_some(colors.background(kind), |row, bg| row.bg(bg))
        .text_color(colors.text)
        .child(gutter(old, colors))
        .child(gutter(new, colors))
        .child(div().flex_none().w(u(px(14.))).child(sign(kind)))
        .child(SharedString::from(text.to_owned()))
        .into_any_element()
}

fn side_row(diff: &ManifestDiff, row_data: SideRow, colors: &Colors) -> AnyElement {
    let (old, new) = match row_data {
        SideRow::Pair { old, new } => (old, new),
        SideRow::Gap(lines) => return gap(lines, colors),
    };
    let half = |line: Option<(u32, LineKind)>, text: &dyn Fn(u32) -> String| {
        let (number, kind) = line.map_or((None, None), |(n, k)| (Some(n), Some(k)));
        h_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_hidden()
            .when_some(kind.and_then(|k| colors.background(k)), |half, bg| {
                half.bg(bg)
            })
            .child(gutter(number, colors))
            .child(
                div()
                    .flex_none()
                    .w(u(px(14.)))
                    .child(kind.map_or(" ", sign)),
            )
            .child(number.map(text).unwrap_or_default())
    };
    row()
        .text_color(colors.text)
        .child(half(old, &|n| diff.old_line(n).to_owned()))
        .child(
            div()
                .flex_none()
                .w(px(1.))
                .h_full()
                .bg(colors.muted.opacity(0.3)),
        )
        .child(half(new, &|n| diff.new_line(n).to_owned()))
        .into_any_element()
}
