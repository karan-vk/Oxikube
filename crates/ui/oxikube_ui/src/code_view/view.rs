//! [`CodeView`]: the entity, the text it is given and the layout work it sends off the UI thread.

use std::sync::Arc;

use gpui::{
    Bounds, Context, FocusHandle, Focusable, Pixels, Task, UniformListScrollHandle, point, px,
};

use super::highlight::{Parsed, StyleCache};
use super::rows::RowMap;
use super::selection::Selection;

/// How a [`CodeView`] looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Look {
    /// The tree-sitter grammar to colour with (`"yaml"`, `"json"`); `None` shows plain text.
    pub language: Option<&'static str>,
    /// Number the lines.
    pub line_numbers: bool,
    /// Wrap long lines at the view's width instead of scrolling sideways.
    pub soft_wrap: bool,
}

impl Look {
    /// Highlighted YAML with line numbers; long lines wrap.
    pub const YAML: Look = Look {
        language: Some("yaml"),
        line_numbers: true,
        soft_wrap: true,
    };

    /// Plain text (`kubectl describe` output): no line numbers, columns stay aligned (long lines
    /// scroll sideways).
    pub const TEXT: Look = Look {
        language: None,
        line_numbers: false,
        soft_wrap: false,
    };
}

/// The text on screen, its rows and (once parsed) its colours.
pub(super) struct Shown {
    pub(super) text: Arc<str>,
    pub(super) rows: Arc<RowMap>,
    pub(super) parsed: Option<Arc<Parsed>>,
}

/// What the last frame measured.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct Metrics {
    /// The advance of one column of the monospace font.
    pub(super) advance: Pixels,
    pub(super) row_height: Pixels,
    /// Space left of the text (after the gutter) and right of it.
    pub(super) padding: Pixels,
    /// The view's bounds (the rows' viewport).
    pub(super) bounds: Option<Bounds<Pixels>>,
    /// The columns the view's width holds (gutter included), once measured.
    pub(super) width_cols: Option<usize>,
}

/// A read-only, virtualised text view. See the [module docs](super).
pub struct CodeView {
    pub(super) look: Look,
    pub(super) focus: FocusHandle,
    pub(super) scroll: UniformListScrollHandle,
    /// The latest text asked for (it may still be being laid out).
    pub(super) wanted: Option<Arc<str>>,
    pub(super) shown: Option<Shown>,
    /// Counts [`CodeView::set_text`] calls: a layout for an older text is dropped.
    generation: u64,
    /// The layout in flight; a newer one replaces (and so cancels) it.
    layout_task: Option<Task<()>>,
    /// The parse in flight for the shown text.
    parse_task: Option<Task<()>>,
    pub(super) metrics: Metrics,
    pub(super) styles: StyleCache,
    pub(super) selection: Selection,
    /// How many rows the last frame built (only the visible ones are).
    pub(super) rows_drawn: usize,
}

impl CodeView {
    /// An empty view looking as `look` says. Give it text with [`CodeView::set_text`].
    pub fn new(look: Look, cx: &mut Context<Self>) -> Self {
        Self {
            look,
            focus: cx.focus_handle(),
            scroll: UniformListScrollHandle::new(),
            wanted: None,
            shown: None,
            generation: 0,
            layout_task: None,
            parse_task: None,
            metrics: Metrics::default(),
            styles: StyleCache::default(),
            selection: Selection::default(),
            rows_drawn: 0,
        }
    }

    /// Shows `text`. Its rows are laid out (and with a language, it is parsed) on a background
    /// thread; the previous text stays on screen until they are ready. The scroll position stays.
    pub fn set_text(&mut self, text: Arc<str>, cx: &mut Context<Self>) {
        if self.wanted.as_ref().is_some_and(|w| Arc::ptr_eq(w, &text)) {
            return;
        }
        self.generation += 1;
        self.wanted = Some(text);
        self.selection = Selection::default();
        self.lay_out(cx);
    }

    /// Shows nothing (the text is no longer valid).
    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        self.wanted = None;
        self.shown = None;
        self.layout_task = None;
        self.parse_task = None;
        self.styles.clear();
        self.selection = Selection::default();
        cx.notify();
    }

    /// Whether a text is on screen (it may be an older one while a newer one is laid out).
    pub fn is_ready(&self) -> bool {
        self.shown.is_some()
    }

    /// Whether the text on screen is the latest one given.
    pub fn is_current(&self) -> bool {
        match (&self.shown, &self.wanted) {
            (Some(shown), Some(wanted)) => Arc::ptr_eq(&shown.text, wanted),
            _ => false,
        }
    }

    /// The text on screen.
    pub fn text(&self) -> Option<&Arc<str>> {
        self.shown.as_ref().map(|shown| &shown.text)
    }

    /// The grammar the view colours with, if any.
    pub fn language(&self) -> Option<&'static str> {
        self.look.language
    }

    /// Whether the text on screen is parsed (its colours are shown).
    pub fn is_highlighted(&self) -> bool {
        self.shown
            .as_ref()
            .is_some_and(|shown| shown.parsed.is_some())
    }

    /// The display rows of the text on screen.
    pub fn row_count(&self) -> usize {
        self.shown.as_ref().map_or(0, |shown| shown.rows.len())
    }

    /// How many rows the last frame built: the visible ones, never the whole text.
    pub fn rows_drawn(&self) -> usize {
        self.rows_drawn
    }

    /// The columns of the view's width the rows were wrapped at (gutter excluded), if wrapped.
    pub fn wrap_cols(&self) -> Option<usize> {
        let shown = self.shown.as_ref()?;
        self.look.soft_wrap.then(|| shown.rows.cols())
    }

    /// The width the rows are laid out for: the measured columns when wrapping (`None` until the
    /// first frame measured them), else unbounded.
    fn layout_width(&self) -> Option<Option<usize>> {
        if self.look.soft_wrap {
            self.metrics.width_cols.map(Some)
        } else {
            Some(None)
        }
    }

    /// Lays the wanted text out off the UI thread. With soft wrap, waits for the first measure.
    fn lay_out(&mut self, cx: &mut Context<Self>) {
        let (Some(text), Some(width)) = (self.wanted.clone(), self.layout_width()) else {
            return;
        };
        let generation = self.generation;
        let gutter = self.look.line_numbers;
        let same_text = self
            .shown
            .as_ref()
            .is_some_and(|shown| Arc::ptr_eq(&shown.text, &text));
        // A text replacing one on screen comes with its colours, so it never flashes uncoloured;
        // the first text shows as soon as its rows are ready and is coloured after.
        let parse_with = match (&self.shown, same_text) {
            (Some(_), false) => self.look.language,
            _ => None,
        };
        self.layout_task = Some(cx.spawn(async move |this, cx| {
            let built = cx
                .background_executor()
                .spawn(async move {
                    let rows = RowMap::build(&text, width, gutter);
                    let parsed = parse_with.map(|language| Parsed::parse(&text, language));
                    (text, rows, parsed)
                })
                .await;
            this.update(cx, |view, cx| view.laid_out(generation, built, cx))
                .ok();
        }));
    }

    fn laid_out(
        &mut self,
        generation: u64,
        (text, rows, parsed): (Arc<str>, RowMap, Option<Parsed>),
        cx: &mut Context<Self>,
    ) {
        if generation != self.generation {
            return;
        }
        let previous = self.shown.take();
        let same_text = previous
            .as_ref()
            .is_some_and(|shown| Arc::ptr_eq(&shown.text, &text));
        // A re-wrap of the same text keeps the line at the top of the view at the top.
        let top_line = previous
            .as_ref()
            .filter(|_| same_text)
            .and_then(|shown| shown.rows.row(self.top_row()))
            .map(|row| row.line);
        let parsed = match parsed {
            Some(parsed) => Some(Arc::new(parsed)),
            None if same_text => previous.and_then(|shown| shown.parsed),
            None => None,
        };
        let needs_parse = parsed.is_none() && !same_text && self.look.language.is_some();
        let rows = Arc::new(rows);
        if let Some(line) = top_line {
            let row = rows.first_row_of_line(line);
            self.set_scroll_y(-(self.metrics.row_height * row as f32));
        }
        self.shown = Some(Shown { text, rows, parsed });
        self.styles.clear();
        if needs_parse {
            self.parse(cx);
        }
        cx.notify();
    }

    /// Parses the text on screen off the UI thread and colours it when done.
    fn parse(&mut self, cx: &mut Context<Self>) {
        let (Some(language), Some(shown)) = (self.look.language, self.shown.as_ref()) else {
            return;
        };
        let text = shown.text.clone();
        self.parse_task = Some(cx.spawn(async move |this, cx| {
            let parse_text = text.clone();
            let parsed = cx
                .background_executor()
                .spawn(async move { Parsed::parse(&parse_text, language) })
                .await;
            this.update(cx, |view, cx| {
                if let Some(shown) = view.shown.as_mut()
                    && Arc::ptr_eq(&shown.text, &text)
                {
                    shown.parsed = Some(Arc::new(parsed));
                    view.styles.clear();
                    cx.notify();
                }
            })
            .ok();
        }));
    }

    /// The frame measured the view: a new width re-wraps the text (off the UI thread).
    pub(super) fn measured(&mut self, bounds: Bounds<Pixels>, cx: &mut Context<Self>) {
        self.metrics.bounds = Some(bounds);
        if !self.look.soft_wrap || self.metrics.advance <= px(0.) {
            return;
        }
        let usable = bounds.size.width - self.metrics.padding * 2. - super::SCROLLBAR_GAP;
        let cols = (usable / self.metrics.advance).floor().max(1.) as usize;
        if self.metrics.width_cols != Some(cols) {
            self.metrics.width_cols = Some(cols);
            self.lay_out(cx);
        }
    }

    /// The row at the top of the viewport.
    pub(super) fn top_row(&self) -> usize {
        let y = -self.scroll_offset().y;
        if self.metrics.row_height <= px(0.) || y <= px(0.) {
            return 0;
        }
        (y / self.metrics.row_height).floor() as usize
    }

    pub(super) fn scroll_offset(&self) -> gpui::Point<Pixels> {
        self.scroll.0.borrow().base_handle.offset()
    }

    /// Scrolls to `y` (0 or negative), clamped to the rows.
    pub(super) fn set_scroll_y(&self, y: Pixels) {
        let rows = self.row_count() as f32;
        let viewport = self.metrics.bounds.map_or(px(0.), |b| b.size.height);
        let lowest = (viewport - self.metrics.row_height * rows).min(px(0.));
        let x = self.scroll_offset().x;
        self.scroll
            .0
            .borrow()
            .base_handle
            .set_offset(point(x, y.clamp(lowest, px(0.))));
    }
}

impl Focusable for CodeView {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}
