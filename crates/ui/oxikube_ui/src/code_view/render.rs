//! Drawing: a `uniform_list` of the rows on screen, each a gutter and one shaped run of text with
//! its colours and the selection, a scrollbar, and a probe that measures the view and follows a
//! drag outside it.

use std::ops::Range;

use gpui::{
    AnyElement, Context, DispatchPhase, HighlightStyle, InteractiveElement as _, IntoElement,
    ListHorizontalSizingBehavior, MouseButton, MouseMoveEvent, MouseUpEvent, ParentElement as _,
    Render, SharedString, Styled as _, StyledText, WeakEntity, Window, canvas, combine_highlights,
    div, font, prelude::FluentBuilder as _, uniform_list,
};
use gpui_component::ActiveTheme as _;
use gpui_component::scroll::Scrollbar;

use super::highlight::{OVERSCAN_ROWS, row_styles};
use super::{
    Bottom, CodeView, Copy, KEY_CONTEXT, LineDown, LineUp, PageDown, PageUp, ROW_HEIGHT, SelectAll,
    Top,
};
use crate::layout::h_flex;
use crate::{ActiveTokens as _, u};

impl Render for CodeView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let family = cx.mono_font_family();
        let font_size = u(tokens.font.mono);
        let row_height = u(ROW_HEIGHT);
        let text_system = window.text_system();
        let font_id = text_system.resolve_font(&font(family.clone()));
        self.metrics.advance = text_system
            .em_advance(font_id, font_size)
            .unwrap_or(font_size * 0.6);
        self.metrics.row_height = row_height;
        self.metrics.padding = u(tokens.spacing.sm);
        self.rows_drawn = 0;

        let view = cx.entity().downgrade();
        let list = self.shown.as_ref().map(|shown| {
            let list = uniform_list(
                "code-view-rows",
                shown.rows.len(),
                cx.processor(|this, range, _, cx| this.render_rows(range, cx)),
            )
            .track_scroll(&self.scroll)
            .size_full();
            if self.look.soft_wrap {
                list.into_any_element()
            } else {
                list.with_horizontal_sizing_behavior(ListHorizontalSizingBehavior::Unconstrained)
                    .with_width_from_item(Some(shown.rows.widest()))
                    .into_any_element()
            }
        });
        let scrollbar = list.is_some().then(|| {
            if self.look.soft_wrap {
                Scrollbar::vertical(&self.scroll)
            } else {
                Scrollbar::new(&self.scroll)
            }
        });
        div()
            .id("code-view")
            .debug_selector(|| "code-view".to_owned())
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &Copy, _, cx| this.copy(cx)))
            .on_action(cx.listener(|this, _: &SelectAll, _, cx| this.select_all(cx)))
            .on_action(cx.listener(|this, _: &LineUp, _, cx| this.scroll_rows(-1., cx)))
            .on_action(cx.listener(|this, _: &LineDown, _, cx| this.scroll_rows(1., cx)))
            .on_action(cx.listener(|this, _: &PageUp, _, cx| {
                let page = this.page_rows();
                this.scroll_rows(-page, cx)
            }))
            .on_action(cx.listener(|this, _: &PageDown, _, cx| {
                let page = this.page_rows();
                this.scroll_rows(page, cx)
            }))
            .on_action(cx.listener(|this, _: &Top, _, cx| this.scroll_to_end(false, cx)))
            .on_action(cx.listener(|this, _: &Bottom, _, cx| this.scroll_to_end(true, cx)))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .relative()
            .size_full()
            .overflow_hidden()
            .font_family(family)
            .text_size(u(tokens.font.mono))
            .line_height(row_height)
            .text_color(tokens.colors.text)
            .child(probe(view, self.selection.dragging))
            .children(list)
            .children(scrollbar)
    }
}

impl CodeView {
    /// The rows `range` of the `uniform_list`: only these are sliced, coloured and shaped.
    fn render_rows(&mut self, range: Range<usize>, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let Some(shown) = self.shown.as_ref() else {
            return Vec::new();
        };
        let rows = shown.rows.clone();
        let text = shown.text.clone();
        let parsed = shown.parsed.clone();
        let (Some(first), Some(last)) =
            (rows.row(range.start), rows.row(range.end.saturating_sub(1)))
        else {
            return Vec::new();
        };
        let tokens = cx.tokens();
        let theme = cx.theme().highlight_theme.clone();
        let styles: &[(Range<usize>, HighlightStyle)] = match parsed.as_deref() {
            Some(parsed) => {
                let wide_first = rows.row(range.start.saturating_sub(OVERSCAN_ROWS));
                let wide_last = rows.row((range.end + OVERSCAN_ROWS).min(rows.len()) - 1);
                let wider = wide_first.map_or(first.start, |r| r.start)
                    ..wide_last.map_or(last.end, |r| r.end);
                self.styles
                    .styles(parsed, &(first.start..last.end), wider, &theme)
            }
            None => &[],
        };
        let selected = self.selection.range();
        let selection = HighlightStyle {
            background_color: Some(tokens.colors.selection),
            ..HighlightStyle::default()
        };
        let found = self
            .matches
            .as_ref()
            .filter(|m| std::sync::Arc::ptr_eq(&m.text, &text))
            .cloned();
        let match_style = HighlightStyle {
            background_color: Some(tokens.colors.warning.opacity(0.35)),
            ..HighlightStyle::default()
        };
        let current_style = HighlightStyle {
            background_color: Some(tokens.colors.accent.opacity(0.6)),
            ..HighlightStyle::default()
        };
        let row_height = self.metrics.row_height;
        let padding = self.metrics.padding;
        let gutter = self
            .look
            .line_numbers
            .then(|| self.metrics.advance * rows.gutter_cols() as f32);
        let elements: Vec<AnyElement> = range
            .filter_map(|ix| {
                let row = rows.row(ix)?;
                let mut runs = row_styles(styles, row.start..row.end);
                if let Some(sel) = selected.as_ref()
                    && sel.start < row.end
                    && row.start < sel.end
                {
                    let span =
                        sel.start.max(row.start) - row.start..sel.end.min(row.end) - row.start;
                    runs = combine_highlights(runs, [(span, selection)]).collect();
                }
                if let Some(found) = found.as_ref() {
                    let first = found.ranges.partition_point(|r| r.end <= row.start);
                    let spans: Vec<(Range<usize>, HighlightStyle)> = found.ranges[first..]
                        .iter()
                        .enumerate()
                        .take_while(|(_, r)| r.start < row.end)
                        .map(|(i, r)| {
                            let style = if found.current == Some(first + i) {
                                current_style
                            } else {
                                match_style
                            };
                            (
                                r.start.max(row.start) - row.start..r.end.min(row.end) - row.start,
                                style,
                            )
                        })
                        .collect();
                    if !spans.is_empty() {
                        runs = combine_highlights(runs, spans).collect();
                    }
                }
                let words = SharedString::from(text.get(row.start..row.end)?.to_owned());
                let number = (gutter.is_some() && rows.starts_line(ix))
                    .then(|| SharedString::from((row.line + 1).to_string()));
                Some(
                    h_flex()
                        .h(row_height)
                        .flex_none()
                        .when_some(gutter, |this, width| {
                            this.child(
                                h_flex()
                                    .flex_none()
                                    .w(width)
                                    .justify_end()
                                    .pr(padding)
                                    .text_color(tokens.colors.text_muted)
                                    .children(number),
                            )
                        })
                        .child(
                            div()
                                .flex_none()
                                .pl(padding)
                                .whitespace_nowrap()
                                .child(StyledText::new(words).with_highlights(runs)),
                        )
                        .into_any_element(),
                )
            })
            .collect();
        self.rows_drawn += elements.len();
        elements
    }
}

/// Measures the view each frame (a new width re-wraps it) and, while a press is held, follows the
/// pointer anywhere in the window so a drag past the edge keeps selecting and scrolls.
fn probe(view: WeakEntity<CodeView>, dragging: bool) -> impl IntoElement {
    let measure = view.clone();
    canvas(
        move |bounds, _, cx| {
            measure
                .update(cx, |this, cx| this.measured(bounds, cx))
                .ok();
        },
        move |_, _, window, _| {
            if !dragging {
                return;
            }
            let moved = view.clone();
            window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                if phase == DispatchPhase::Bubble && event.pressed_button == Some(MouseButton::Left)
                {
                    moved
                        .update(cx, |this, cx| this.drag_to(event.position, cx))
                        .ok();
                }
            });
            window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                if phase == DispatchPhase::Bubble && event.button == MouseButton::Left {
                    view.update(cx, |this, _| this.end_drag()).ok();
                }
            });
        },
    )
    .absolute()
    .size_full()
}
