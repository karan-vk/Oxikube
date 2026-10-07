//! The rows: a line (timestamp and text, coloured by level), the "truncated" marker and the state
//! row. Only the rows on screen are built, from the session's ring buffer under one short lock
//! per frame (per row, wrapped).

use std::ops::Range;
use std::sync::Arc;

use gpui::{
    AnyElement, Context, FontWeight, HighlightStyle, Hsla, IntoElement, ParentElement as _,
    SharedString, Styled as _, StyledText, div,
};
use oxikube_app::logs::{LogEntry, LogState};
use oxikube_domain::log::LogLevel;
use oxikube_ui::layout::h_flex;
use oxikube_ui::{ActiveTokens as _, Colors, u};

use super::highlight::{LineMarks, Mark};
use super::json::JsonColumns;
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
        /// How the search marks the row, and the byte ranges of its matches to highlight.
        marks: LineMarks,
        /// Whether the user marked the line (`m`): the gutter bar.
        marked: bool,
        /// Whether the line is in the selection.
        selected: bool,
    },
    /// A structured line in JSON mode: columns instead of text.
    Json {
        seq: u64,
        ts: Option<SharedString>,
        record: Arc<JsonColumns>,
        expanded: bool,
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

    /// The words row `index` shows (timestamp and text for a line; for a structured line in JSON
    /// mode its level, time, message and collapsed fields), as drawn.
    pub fn row_text(&self, index: usize) -> Option<String> {
        let row = self.window.row(index)?;
        let data = self.row_data(&[Some(row)]).into_iter().next()?;
        Some(match data {
            RowData::Line {
                ts: Some(ts), text, ..
            } => format!("{ts} {text}"),
            RowData::Line { ts: None, text, .. } => text.to_string(),
            RowData::Json { ts, record, .. } => {
                let level = record.level.label().to_uppercase();
                let parts = [
                    ts.as_deref().unwrap_or(""),
                    &level,
                    &record.time,
                    &record.message,
                    &record.summary,
                ];
                parts
                    .into_iter()
                    .filter(|part| !part.is_empty())
                    .collect::<Vec<_>>()
                    .join(" ")
            }
            RowData::Gone => String::new(),
            RowData::Marker(words) | RowData::State(words, _) => words.to_string(),
        })
    }

    /// The columns of row `index` when it is a structured line drawn in JSON mode.
    pub fn row_columns(&self, index: usize) -> Option<Arc<JsonColumns>> {
        let row = self.window.row(index)?;
        match self.row_data(&[Some(row)]).into_iter().next()? {
            RowData::Json { record, .. } => Some(record),
            _ => None,
        }
    }

    /// Lines parsed for drawing so far: the columns of the rows that were on screen, each once
    /// (the cache serves the rest), never the whole buffer.
    pub fn parsed_rows(&self) -> usize {
        self.records.borrow().parsed()
    }

    /// The original text of the line at row `index`, exactly as the container wrote it (what a
    /// copy takes, never the reconstructed columns).
    pub fn raw_text(&self, index: usize) -> Option<Arc<str>> {
        let Some(Row::Line(seq)) = self.window.row(index) else {
            return None;
        };
        self.session
            .as_ref()?
            .read(|buffer, _| buffer.get_seq(seq).map(|entry| entry.text.clone()))
    }

    /// The rows' data with the search's marks on the lines, computed after the session's lock
    /// was let go (a regex over the few rows on screen, not under the lock).
    fn row_data(&self, rows: &[Option<Row>]) -> Vec<RowData> {
        let mut data = self.read_rows(rows);
        for row in &mut data {
            if let RowData::Line {
                seq, text, marks, ..
            } = row
            {
                *marks = self.line_marks(*seq, text);
            }
        }
        data
    }

    fn read_rows(&self, rows: &[Option<Row>]) -> Vec<RowData> {
        let timestamps = self.options.timestamps;
        let wrap = self.options.wrap;
        let capacity = self.deps.service.buffer_lines();
        let state = self.window.state();
        let json = self.options.json;
        let expanded = self.expanded.as_ref().map(|e| e.seq);
        let line = |entry: Option<&LogEntry>| match entry {
            Some(entry) => {
                let ts = timestamps.then(|| SharedString::from(timestamp(entry)));
                let record = (json && entry.level.is_some())
                    .then(|| self.records.borrow_mut().row(entry.seq, &entry.text))
                    .flatten();
                let marked = self.marks.contains(entry.seq);
                let selected = self.selection.contains(entry.seq);
                match record {
                    Some(record) => RowData::Json {
                        seq: entry.seq,
                        ts,
                        record,
                        expanded: expanded == Some(entry.seq),
                        marked,
                        selected,
                    },
                    None => RowData::Line {
                        seq: entry.seq,
                        ts,
                        level: level_of(&entry.text),
                        text: line_text(entry, wrap),
                        marks: LineMarks::default(),
                        marked,
                        selected,
                    },
                }
            }
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
                marks,
                marked,
                selected,
            } => {
                let base = match marks.mark {
                    Mark::None => base,
                    Mark::Matched => base.bg(colors.element),
                    Mark::Current => base.bg(colors.element_selected),
                };
                let text = div()
                    .flex_1()
                    .min_w_0()
                    .text_color(level_colour(level, &colors))
                    .child(highlighted(
                        text,
                        &marks.spans,
                        colors.warning.opacity(0.45),
                    ));
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
                self.row_chrome(base, seq, marked, selected, cx)
                    .children(ts)
                    .child(text)
                    .into_any_element()
            }
            RowData::Json {
                seq,
                ts,
                record,
                expanded,
                marked,
                selected,
            } => {
                let base = self.row_chrome(base, seq, marked, selected, cx);
                self.json_row(base, seq, ts, &record, expanded, wrap, cx)
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

/// `text`, with the byte ranges `spans` highlighted in `colour`.
fn highlighted(text: SharedString, spans: &[Range<usize>], colour: Hsla) -> AnyElement {
    if spans.is_empty() {
        return text.into_any_element();
    }
    let style = HighlightStyle {
        background_color: Some(colour),
        ..HighlightStyle::default()
    };
    StyledText::new(text)
        .with_highlights(spans.iter().map(|span| (span.clone(), style)))
        .into_any_element()
}

/// The accent of a level: its chip, and the title of the expanded pane.
pub(super) fn level_accent(level: LogLevel, colors: &Colors) -> Hsla {
    match level {
        LogLevel::Fatal | LogLevel::Error => colors.error,
        LogLevel::Warn => colors.warning,
        LogLevel::Info => colors.info,
        LogLevel::Debug | LogLevel::Trace | LogLevel::Unknown => colors.text_muted,
    }
}
