//! [`RowCache`]: laid-out and shaped rows, reused for any row whose content was on screen in the
//! previous frame.
//!
//! Rows are keyed by a hash of their cells and combining marks, not by their position: a line
//! that scrolls up keeps its shaped runs, identical rows (blank lines) share one entry, and an
//! idle repaint (cursor blink, hover) shapes nothing. So `yes` or a scrolling log shapes one new
//! line per frame, and a full redraw (`htop`) only the rows that differ. What the hash does not
//! cover drops every entry at once: the font, the cell metrics, the palette, the column count.
//! Entries not shown in a frame are recycled (their buffers kept) for the next misses.

use std::collections::HashMap;
use std::sync::Arc;

use gpui::{ShapedLine, SharedString, TextRun, WindowTextSystem};

use super::hash::{FxBuild, row_hash};
use super::layout::{RowLayout, TextSpan};
use super::metrics::{CellMetrics, TerminalFont};
use super::palette::TerminalPalette;
use crate::grid::TerminalSnapshot;

/// One cached row.
#[derive(Default)]
pub(super) struct CachedRow {
    /// What the row paints.
    pub layout: RowLayout,
    /// One shaped line per [`RowLayout::runs`] entry.
    pub shaped: Vec<ShapedLine>,
}

/// Counters for tests and `--perf`: how many rows were reused or rebuilt, and how many runs were
/// shaped.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CacheStats {
    /// Rows painted from the cache.
    pub hits: u64,
    /// Rows laid out and shaped again.
    pub misses: u64,
    /// Text runs shaped (the work the cache saves).
    pub shaped_runs: u64,
}

/// What invalidates every row at once.
#[derive(Clone, PartialEq)]
struct CacheKey {
    font: TerminalFont,
    metrics: CellMetrics,
    palette: TerminalPalette,
    columns: usize,
}

/// See the [module docs](self).
#[derive(Default)]
pub(super) struct RowCache {
    key: Option<CacheKey>,
    /// This frame's rows by content hash.
    current: HashMap<u64, CachedRow, FxBuild>,
    /// The previous frame's rows, while a frame is being built.
    previous: HashMap<u64, CachedRow, FxBuild>,
    /// The content hash of each viewport row, top to bottom.
    order: Vec<u64>,
    /// Rows no longer shown, kept for their buffers.
    spare: Vec<CachedRow>,
    stats: CacheStats,
}

impl RowCache {
    /// Hit and miss counters since the cache was made.
    pub fn stats(&self) -> CacheStats {
        self.stats
    }

    /// The viewport rows, top to bottom, after [`update`](Self::update).
    pub fn rows(&self) -> impl Iterator<Item = &CachedRow> {
        self.order.iter().filter_map(|hash| self.current.get(hash))
    }

    /// Brings every viewport row of `snapshot` up to date, laying out and shaping only content
    /// the previous frame did not show.
    pub fn update(
        &mut self,
        snapshot: &TerminalSnapshot,
        font: &TerminalFont,
        metrics: CellMetrics,
        palette: &TerminalPalette,
        text_system: &WindowTextSystem,
    ) {
        let stale = self.key.as_ref().is_none_or(|key| {
            key.columns != snapshot.columns
                || key.metrics != metrics
                || key.font != *font
                || key.palette != *palette
        });
        if stale {
            self.key = Some(CacheKey {
                font: font.clone(),
                metrics,
                palette: palette.clone(),
                columns: snapshot.columns,
            });
            self.spare.extend(self.current.drain().map(|(_, row)| row));
        }
        std::mem::swap(&mut self.current, &mut self.previous);
        self.order.clear();
        for index in 0..snapshot.rows {
            let hash = row_hash(snapshot, index);
            self.order.push(hash);
            if self.current.contains_key(&hash) {
                self.stats.hits += 1;
                continue;
            }
            if let Some(row) = self.previous.remove(&hash) {
                self.stats.hits += 1;
                self.current.insert(hash, row);
                continue;
            }
            self.stats.misses += 1;
            let mut row = self.spare.pop().unwrap_or_default();
            row.layout.build(snapshot, index, palette);
            row.shaped.clear();
            for run in &row.layout.runs {
                row.shaped
                    .push(shape(&row.layout, run, font, metrics, text_system));
                self.stats.shaped_runs += 1;
            }
            self.current.insert(hash, row);
        }
        let keep = snapshot.rows;
        for (_, row) in self.previous.drain() {
            if self.spare.len() < keep {
                self.spare.push(row);
            }
        }
    }
}

/// Shapes one run. Every glyph is forced to one cell, so a run never drifts off the grid; a wide
/// glyph is a run of its own and keeps its natural width.
fn shape(
    layout: &RowLayout,
    run: &TextSpan,
    font: &TerminalFont,
    metrics: CellMetrics,
    text_system: &WindowTextSystem,
) -> ShapedLine {
    let text = &layout.text[run.bytes.clone()];
    let style = TextRun {
        len: text.len(),
        font: font.font(run.bold, run.italic),
        color: run.color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let force = (!run.wide).then_some(metrics.cell_width);
    // `Arc<str>` straight from the slice: one allocation, not a `String` and then an `Arc`.
    text_system.shape_line(
        SharedString::from(Arc::<str>::from(text)),
        metrics.font_size,
        &[style],
        force,
    )
}
