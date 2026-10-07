//! The rows: a line (timestamp and text, coloured by level), the "truncated" marker and the state
//! row. Only the rows on screen are built, from the session's ring buffer under one short lock
//! per frame (per row, wrapped).

use std::ops::Range;

use gpui::{
    AnyElement, Context, FontWeight, Hsla, InteractiveElement as _, IntoElement, MouseButton,
    MouseDownEvent, MouseMoveEvent, ParentElement as _, SharedString, Styled as _, Window, div, px,
};
use oxikube_app::logs::{LogEntry, LogState};
use oxikube_ui::layout::h_flex;
use oxikube_ui::{ActiveTokens as _, Colors, u};

use super::text::{Level, level_of, state_text, timestamp, truncated_marker};
use super::window::Row;
use super::{LogView, NOWRAP_CHARS};

/// What one row shows, read out of the buffer before any element is built.
enum RowData {
    Line {
        seq: u64,
        ts: Option<SharedString>,
        text: SharedString,
        level: Level,
        marked: bool,
        selected: bool,
    },
    /// A line the buffer dropped since the last delta (drawn empty for that frame).
    Gone,
    Marker(SharedString),
    State(SharedString, bool),
}

impl LogView {
    /// The unwrapped rows of `range` (the `uniform_list`'s request).
    pub(crate) fn render_rows(
        &mut self,
        range: Range<usize>,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let rows: Vec<Option<Row>> = range.map(|ix| self.window.row(ix)).collect();
        let data = self.row_data(&rows);
        self.rows_built += data.len();
        data.into_iter()
            .map(|row| self.row_element(row, false, cx))
            .collect()
    }

    /// The wrapped row `ix` (the `list`'s request).
    pub(crate) fn render_wrapped(&mut self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let data = self.row_data(&[self.window.row(ix)]);
        self.rows_built += 1;
        let row = data.into_iter().next().unwrap_or(RowData::Gone);
        self.row_element(row, true, cx)
    }

    /// The words row `index` shows (timestamp and text for a line), as drawn.
    pub fn row_text(&self, index: usize) -> Option<String> {
        let row = self.window.row(index)?;
        let data = self.row_data(&[Some(row)]).into_iter().next()?;
        Some(match data {
            RowData::Line {
                ts: Some(ts), text, ..
            } => format!("{ts} {text}"),
            RowData::Line { ts: None, text, .. } => text.to_string(),
            RowData::Gone => String::new(),
            RowData::Marker(words) | RowData::State(words, _) => words.to_string(),
        })
    }

    fn row_data(&self, rows: &[Option<Row>]) -> Vec<RowData> {
        let timestamps = self.options.timestamps;
        let wrap = self.options.wrap;
        let capacity = self.deps.service.buffer_lines();
        let state = self.window.state();
        let line = |entry: Option<&LogEntry>| match entry {
            Some(entry) => RowData::Line {
                seq: entry.seq,
                ts: timestamps.then(|| timestamp(entry).into()),
                level: level_of(&entry.text),
                text: line_text(entry, wrap),
                marked: self.marks.contains(entry.seq),
                selected: self.selection.contains(entry.seq),
            },
            None => RowData::Gone,
        };
        let build = |row: &Option<Row>, entry: Option<&LogEntry>| match row {
            Some(Row::Line(_)) => line(entry),
            Some(Row::Truncated(dropped)) => {
                RowData::Marker(truncated_marker(*dropped, capacity).into())
            }
            Some(Row::State) | None => RowData::State(
                state_text(state).into(),
                matches!(state, LogState::Failed(_)),
            ),
        };
        match &self.session {
            Some(session) => session.read(|buffer, _| {
                rows.iter()
                    .map(|row| {
                        let entry = match row {
                            Some(Row::Line(seq)) => buffer.get_seq(*seq),
                            _ => None,
                        };
                        build(row, entry)
                    })
                    .collect()
            }),
            None => rows.iter().map(|row| build(row, None)).collect(),
        }
    }

    fn row_element(&self, row: RowData, wrap: bool, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let base = h_flex()
            .w_full()
            .px(u(tokens.spacing.md))
            .gap(u(tokens.spacing.md))
            .font_family(cx.mono_font_family())
            .text_size(u(tokens.font.mono))
            .line_height(self.row_height());
        let base = if wrap {
            base.items_start()
        } else {
            base.h(self.row_height()).items_center().overflow_hidden()
        };
        match row {
            RowData::Line {
                seq,
                ts,
                text,
                level,
                marked,
                selected,
            } => {
                let text = div()
                    .flex_1()
                    .min_w_0()
                    .text_color(level_colour(level, &colors))
                    .child(text);
                let text = if wrap {
                    text
                } else {
                    text.whitespace_nowrap().overflow_hidden()
                };
                let ts = ts.map(|ts| {
                    div()
                        .flex_none()
                        .whitespace_nowrap()
                        .text_color(colors.text_muted)
                        .child(ts)
                });
                // The gutter: a bar on the left edge of a marked line, over the row's padding so
                // marking a line moves nothing.
                let gutter = marked.then(|| {
                    div()
                        .absolute()
                        .left_0()
                        .top_0()
                        .bottom_0()
                        .w(u(px(3.)))
                        .bg(colors.warning)
                        .debug_selector(move || format!("log-mark:{seq}"))
                });
                base.id(("log-row", seq as usize))
                    .relative()
                    .bg(if selected {
                        colors.selection
                    } else {
                        gpui::transparent_black()
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(
                            move |view, event: &MouseDownEvent, window: &mut Window, cx| {
                                window.focus(&view.focus, cx);
                                view.click_line(seq, event.modifiers.shift, cx);
                            },
                        ),
                    )
                    .on_mouse_move(cx.listener(move |view, event: &MouseMoveEvent, _, cx| {
                        if event.dragging() {
                            view.drag_to_line(seq, cx);
                        }
                    }))
                    .children(gutter)
                    .children(ts)
                    .child(text)
                    .into_any_element()
            }
            RowData::Gone => base.into_any_element(),
            RowData::Marker(words) => base
                .bg(colors.surface)
                .text_color(colors.warning)
                .child(div().whitespace_nowrap().child(words))
                .into_any_element(),
            RowData::State(words, failed) => base
                .text_color(if failed {
                    colors.error
                } else {
                    colors.text_muted
                })
                .font_weight(FontWeight::MEDIUM)
                .child(div().whitespace_nowrap().child(words))
                .into_any_element(),
        }
    }
}

/// The text a row draws: all of it wrapped, the first [`NOWRAP_CHARS`] bytes unwrapped. Shares
/// the buffer's `Arc<str>` (no copy) unless it is cut.
fn line_text(entry: &LogEntry, wrap: bool) -> SharedString {
    if wrap || entry.text.len() <= NOWRAP_CHARS {
        return SharedString::from(entry.text.clone());
    }
    let mut end = NOWRAP_CHARS;
    while !entry.text.is_char_boundary(end) {
        end -= 1;
    }
    SharedString::from(entry.text[..end].to_owned())
}

fn level_colour(level: Level, colors: &Colors) -> Hsla {
    match level {
        Level::Error => colors.error,
        Level::Warn => colors.warning,
        Level::Debug => colors.text_muted,
        Level::Plain => colors.text,
    }
}
